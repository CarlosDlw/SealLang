//! Writing object, assembly, and executable artifacts.
//!
//! Object and assembly output are written atomically through `seal-fs`. An
//! executable is linked from a temporary object and the embedded runtime
//! archive in the output directory into a temporary executable, which replaces
//! the destination only after a successful link. Every temporary file is
//! removed on every path, including linker failure.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use inkwell::module::Module;
use inkwell::targets::{FileType, TargetMachine};

use crate::error::CodegenError;
use crate::linker;
use crate::options::{CodegenOptions, Emit};
use seal_target::RuntimePolicy;

/// Produces the requested artifact for `module`.
pub(crate) fn emit_artifact(
    module: &Module<'_>,
    machine: &TargetMachine,
    options: &CodegenOptions,
    runtime: Option<&[u8]>,
    source_libraries: &[String],
) -> Result<(), CodegenError> {
    match options.emit {
        Emit::Object => write_buffer(module, machine, FileType::Object, &options.output),
        Emit::Assembly => write_buffer(module, machine, FileType::Assembly, &options.output),
        Emit::Executable => link_executable(module, machine, options, runtime, source_libraries),
        Emit::StaticLib => link_staticlib(module, machine, options, runtime),
        Emit::Cdylib => link_cdylib(module, machine, options, runtime, source_libraries),
        Emit::Mono => Err(CodegenError::internal(
            "a mono artifact is written by the mono backend, not the artifact writer",
        )),
    }
}

fn write_buffer(
    module: &Module<'_>,
    machine: &TargetMachine,
    file_type: FileType,
    output: &Path,
) -> Result<(), CodegenError> {
    let buffer = machine
        .write_to_memory_buffer(module, file_type)
        .map_err(|error| {
            CodegenError::artifact(
                output,
                format!("LLVM could not generate the artifact: {error}"),
            )
        })?;
    seal_fs::write_atomic(output, buffer.as_slice()).map_err(|error| {
        CodegenError::artifact(output, format!("cannot write the artifact: {error}"))
    })
}

fn link_executable(
    module: &Module<'_>,
    machine: &TargetMachine,
    options: &CodegenOptions,
    runtime: Option<&[u8]>,
    source_libraries: &[String],
) -> Result<(), CodegenError> {
    link_module(module, machine, options, runtime, source_libraries, false)
}

fn link_cdylib(
    module: &Module<'_>,
    machine: &TargetMachine,
    options: &CodegenOptions,
    runtime: Option<&[u8]>,
    source_libraries: &[String],
) -> Result<(), CodegenError> {
    link_module(module, machine, options, runtime, source_libraries, true)
}

fn link_staticlib(
    module: &Module<'_>,
    machine: &TargetMachine,
    options: &CodegenOptions,
    runtime: Option<&[u8]>,
) -> Result<(), CodegenError> {
    let output = options.output.as_path();
    let object = machine
        .write_to_memory_buffer(module, FileType::Object)
        .map_err(|error| {
            CodegenError::artifact(
                output,
                format!("LLVM could not generate an object file: {error}"),
            )
        })?;
    let directory = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = output
        .file_name()
        .ok_or_else(|| CodegenError::artifact(output, "the output path has no file name"))?;
    let object_path = directory.join(format!(
        ".{}.{}.o",
        file_name.to_string_lossy(),
        std::process::id()
    ));
    let _object_guard = TempFile(object_path.clone());
    seal_fs::write_atomic(&object_path, object.as_slice()).map_err(|error| {
        CodegenError::artifact(
            &object_path,
            format!("cannot write the temporary object file: {error}"),
        )
    })?;
    archive_files(&[object_path.as_path()], runtime, output)
}

fn link_module(
    module: &Module<'_>,
    machine: &TargetMachine,
    options: &CodegenOptions,
    runtime: Option<&[u8]>,
    source_libraries: &[String],
    shared: bool,
) -> Result<(), CodegenError> {
    let output = options.output.as_path();
    let object = machine
        .write_to_memory_buffer(module, FileType::Object)
        .map_err(|error| {
            CodegenError::artifact(
                output,
                format!("LLVM could not generate an object file: {error}"),
            )
        })?;
    let directory = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = output
        .file_name()
        .ok_or_else(|| CodegenError::artifact(output, "the output path has no file name"))?;
    let object_path = directory.join(format!(
        ".{}.{}.o",
        file_name.to_string_lossy(),
        std::process::id()
    ));
    let _object_guard = TempFile(object_path.clone());
    seal_fs::write_atomic(&object_path, object.as_slice()).map_err(|error| {
        CodegenError::artifact(
            &object_path,
            format!("cannot write the temporary object file: {error}"),
        )
    })?;
    let objects = [object_path.as_path()];
    let request = LinkRequest {
        objects: &objects,
        runtime,
        output,
        shared,
        source_libraries,
        libraries: &options.libraries,
        search_paths: &options.search_paths,
        link_args: &options.link_args,
        linker: options.linker.as_deref().map(std::ffi::OsStr::new),
        freestanding: options.runtime == RuntimePolicy::None,
        target: options.target_triple.as_deref().unwrap_or_default(),
        linker_script: options.linker_script.as_deref(),
        sysroot: options.sysroot.as_deref(),
        source_date_epoch: options.source_date_epoch,
        reproducible: options.reproducible,
    };
    run_linker(&request)
}

/// Archives object files and the runtime into a deterministic static library.
///
/// On a GNU-like host the runtime archive is partially linked into one
/// relocatable object and `ar crsD` stores that single member. On MSVC the
/// `lib` tool combines the inputs. The destination is replaced only after the
/// whole archive exists.
pub(crate) fn archive_files(
    objects: &[&Path],
    runtime: Option<&[u8]>,
    output: &Path,
) -> Result<(), CodegenError> {
    let directory = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = output
        .file_name()
        .ok_or_else(|| CodegenError::artifact(output, "the output path has no file name"))?;
    let serial = TEMPORARY.fetch_add(1, Ordering::Relaxed);
    let stem = format!(
        "{}.{}.{serial}",
        file_name.to_string_lossy(),
        std::process::id()
    );
    // The member name is what `ar` stores. A stable `combined.o` inside a
    // unique directory keeps the archive bytes independent of the process id
    // and of concurrent builds.
    let work = directory.join(format!(".{stem}.archive-work"));
    std::fs::create_dir_all(&work).map_err(|error| {
        CodegenError::artifact(
            &work,
            format!("cannot create the archive work directory: {error}"),
        )
    })?;
    let _work_guard = TempDirectory(work.clone());
    let runtime_path = work.join("runtime.a");
    // A fresh directory also guarantees that `ar` never appends to a stale
    // archive left by a process that reused the same pid.
    let temporary = work.join("archive.a");
    if let Some(runtime) = runtime {
        seal_fs::write_atomic(&runtime_path, runtime).map_err(|error| {
            CodegenError::artifact(
                &runtime_path,
                format!("cannot write the temporary runtime archive: {error}"),
            )
        })?;
    }

    if cfg!(windows) {
        let program = std::env::var_os("SEAL_AR").unwrap_or_else(|| OsString::from("lib"));
        let mut command = Command::new(&program);
        command.arg("/NOLOGO").arg(prefixed("/OUT:", &temporary));
        for object in objects {
            command.arg(object);
        }
        if runtime.is_some() {
            command.arg(&runtime_path);
        }
        run_tool(command, &program, output)?;
    } else {
        let combined = work.join("combined.o");
        let compiler = std::env::var_os("CC").unwrap_or_else(|| OsString::from("cc"));
        let mut partial = Command::new(&compiler);
        partial.arg("-r").arg("-o").arg(&combined);
        for object in objects {
            partial.arg(object);
        }
        if runtime.is_some() {
            partial.arg(&runtime_path);
        }
        run_tool(partial, &compiler, output)?;

        let program = std::env::var_os("SEAL_AR").unwrap_or_else(|| OsString::from("ar"));
        let mut archive = Command::new(&program);
        if cfg!(any(target_os = "linux", target_os = "android")) {
            archive.arg("crsD");
        } else {
            // BSD and Apple tools spell deterministic mode through the
            // environment instead of the GNU `D` modifier.
            archive.arg("crs").env("ZERO_AR_DATE", "1");
        }
        archive.arg(&temporary).arg(&combined);
        run_tool(archive, &program, output)?;
    }

    seal_fs::replace(&temporary, output).map_err(|error| {
        CodegenError::artifact(
            output,
            format!("cannot replace the output with the static library: {error}"),
        )
    })
}

fn run_tool(
    mut command: Command,
    program: &std::ffi::OsStr,
    output: &Path,
) -> Result<(), CodegenError> {
    let result = command.output();
    match result {
        Ok(result) if result.status.success() => Ok(()),
        Ok(result) => Err(CodegenError::artifact(
            output,
            format!(
                "`{}` failed ({}): {}",
                program.to_string_lossy(),
                result.status,
                String::from_utf8_lossy(&result.stderr).trim()
            ),
        )),
        Err(error) => Err(CodegenError::artifact(
            output,
            format!("cannot run `{}`: {error}", program.to_string_lossy()),
        )),
    }
}

fn prefixed(prefix: &str, path: &Path) -> OsString {
    let mut argument = OsString::from(prefix);
    argument.push(path);
    argument
}

/// One link request over an existing object file.
pub(crate) struct LinkRequest<'a> {
    pub(crate) objects: &'a [&'a Path],
    pub(crate) runtime: Option<&'a [u8]>,
    pub(crate) output: &'a Path,
    pub(crate) shared: bool,
    pub(crate) source_libraries: &'a [String],
    pub(crate) libraries: &'a [String],
    pub(crate) search_paths: &'a [PathBuf],
    pub(crate) link_args: &'a [OsString],
    pub(crate) linker: Option<&'a std::ffi::OsStr>,
    pub(crate) freestanding: bool,
    pub(crate) target: &'a str,
    pub(crate) linker_script: Option<&'a Path>,
    pub(crate) sysroot: Option<&'a Path>,
    pub(crate) source_date_epoch: u64,
    pub(crate) reproducible: bool,
}

/// Links one object into `output`, replacing it atomically.
pub(crate) fn run_linker(request: &LinkRequest<'_>) -> Result<(), CodegenError> {
    let output = request.output;
    let directory = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = output
        .file_name()
        .ok_or_else(|| CodegenError::artifact(output, "the output path has no file name"))?;
    let runtime_path = directory.join(format!(
        ".{}.{}.{}.seal_rt{}",
        file_name.to_string_lossy(),
        std::process::id(),
        LINK_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        if cfg!(windows) { ".lib" } else { ".a" }
    ));
    let temporary_path = temporary_executable(output)?;
    let mut objects: Vec<PathBuf> = request
        .objects
        .iter()
        .map(|path| path.to_path_buf())
        .collect();
    let mut support_guard = None;
    if request.freestanding {
        let work = directory.join(format!(
            ".{}.{}.{}.freestanding",
            file_name.to_string_lossy(),
            std::process::id(),
            LINK_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let guard = TempDirectory(work.clone());
        let stub = compile_exit_stub(&work, request.target, request.linker)?;
        objects.push(stub);
        support_guard = Some(guard);
    }
    let mut link_args: Vec<OsString> = request.link_args.to_vec();
    if request.reproducible
        && !request.shared
        && matches!(
            request.target.split('-').nth(2).unwrap_or_default(),
            "linux" | "freebsd" | "android"
        )
    {
        link_args.push(OsString::from("-Wl,--build-id=none"));
    }
    let runtime = request.runtime.map(|_| runtime_path.as_path());
    let inputs = linker::LinkInputs {
        objects,
        runtime,
        output: &temporary_path,
        shared: request.shared,
        source_libraries: request.source_libraries,
        libraries: request.libraries,
        search_paths: request.search_paths,
        link_args: &link_args,
        freestanding: request.freestanding,
        linker_script: request.linker_script,
        sysroot: request.sysroot,
    };
    // Validate before writing the runtime archive or executable: a bad `-l` or
    // `-L` must not produce a partial output. The guarded support directory is
    // removed on every early return.
    linker::validate(&inputs)?;

    let _runtime_guard = if let Some(runtime) = request.runtime {
        let guard = TempFile(runtime_path.clone());
        seal_fs::write_atomic(&runtime_path, runtime).map_err(|error| {
            CodegenError::artifact(
                &runtime_path,
                format!("cannot write the temporary runtime archive: {error}"),
            )
        })?;
        Some(guard)
    } else {
        None
    };
    let _temporary_guard = TempFile(temporary_path.clone());
    let _support_guard = support_guard;
    let program = request
        .linker
        .map(std::ffi::OsStr::to_owned)
        .unwrap_or_else(linker::default_program);
    let invocation = linker::invocation(program, &inputs);
    let result = Command::new(invocation.program())
        .args(invocation.arguments())
        .env("SOURCE_DATE_EPOCH", request.source_date_epoch.to_string())
        .output();
    match result {
        Ok(result) if result.status.success() => {
            // The destination is replaced only after a successful link, so a
            // failed link never truncates the output of an earlier build.
            seal_fs::replace(&temporary_path, output).map_err(|error| {
                CodegenError::artifact(
                    output,
                    format!("cannot replace the output with the linked executable: {error}"),
                )
            })
        }
        Ok(result) => Err(CodegenError::artifact(
            output,
            format!(
                "linker `{}` failed ({}): {}\ncommand: {}\nlibraries: {}",
                invocation.program().to_string_lossy(),
                result.status,
                String::from_utf8_lossy(&result.stderr).trim(),
                invocation.command_line(),
                libraries_text(&inputs),
            ),
        )),
        Err(error) => Err(CodegenError::linker_input(format!(
            "cannot run linker `{}` for target `{}`: {error}; declare a linker for this target",
            invocation.program().to_string_lossy(),
            request.target
        ))),
    }
}

/// Compiles the freestanding `exit` implementation used by `_start`.
///
/// The implementation uses the target's direct process-exit system call, so it
/// needs no libc and no runtime archive. `SEAL_CC`, then the declared linker
/// driver, then `CC`, then `cc` compiles it. A cross build therefore uses the
/// same target compiler driver as the link unless `SEAL_CC` overrides it.
fn compile_exit_stub(
    work: &Path,
    target: &str,
    linker: Option<&std::ffi::OsStr>,
) -> Result<PathBuf, CodegenError> {
    std::fs::create_dir_all(work).map_err(|error| {
        CodegenError::artifact(
            work,
            format!("cannot create the freestanding support directory: {error}"),
        )
    })?;
    let source = work.join("exit.c");
    std::fs::write(&source, FREESTANDING_EXIT).map_err(|error| {
        CodegenError::artifact(
            &source,
            format!("cannot write the freestanding exit stub: {error}"),
        )
    })?;
    let object = work.join("exit.o");
    let compiler = std::env::var_os("SEAL_CC")
        .or_else(|| linker.map(std::ffi::OsStr::to_owned))
        .or_else(|| std::env::var_os("CC"))
        .unwrap_or_else(|| OsString::from("cc"));
    let result = Command::new(&compiler)
        .arg("-ffreestanding")
        .arg("-fno-builtin")
        .arg("-fno-stack-protector")
        .arg("-c")
        .arg(&source)
        .arg("-o")
        .arg(&object)
        .env("SOURCE_DATE_EPOCH", "0")
        .output();
    match result {
        Ok(result) if result.status.success() => Ok(object),
        Ok(result) => Err(CodegenError::linker_input(format!(
            "cannot compile the freestanding exit stub for `{target}` with `{}`: {}; declare a C compiler for the target in `SEAL_CC`",
            compiler.to_string_lossy(),
            String::from_utf8_lossy(&result.stderr).trim()
        ))),
        Err(error) => Err(CodegenError::linker_input(format!(
            "cannot run the C compiler `{}` for the freestanding target `{target}`: {error}",
            compiler.to_string_lossy()
        ))),
    }
}

const FREESTANDING_EXIT: &str = r#"
void exit(int code);

#if defined(__x86_64__)
#  if defined(__APPLE__)
#    define SEAL_EXIT_SYSCALL 0x2000001
#  else
#    define SEAL_EXIT_SYSCALL 60
#  endif
void exit(int code) {
  __asm__ __volatile__("syscall" :: "a"(SEAL_EXIT_SYSCALL), "D"((long)code) : "rcx", "r11", "memory");
  __builtin_unreachable();
}
#elif defined(__aarch64__)
#  if defined(__APPLE__)
#    define SEAL_EXIT_SYSCALL 1
#  else
#    define SEAL_EXIT_SYSCALL 93
#  endif
void exit(int code) {
  register long x8 __asm__("x8") = SEAL_EXIT_SYSCALL;
  register long x0 __asm__("x0") = code;
  __asm__ __volatile__("svc #0" :: "r"(x8), "r"(x0) : "memory");
  __builtin_unreachable();
}
#elif defined(__i386__)
void exit(int code) {
  __asm__ __volatile__("int $0x80" :: "a"(1), "b"(code) : "memory");
  __builtin_unreachable();
}
#elif defined(__arm__)
void exit(int code) {
  register long r7 __asm__("r7") = 1;
  register long r0 __asm__("r0") = code;
  __asm__ __volatile__("svc #0" :: "r"(r7), "r"(r0) : "memory");
  __builtin_unreachable();
}
#elif defined(__riscv)
void exit(int code) {
  register long a7 __asm__("a7") = 93;
  register long a0 __asm__("a0") = code;
  __asm__ __volatile__("ecall" :: "r"(a7), "r"(a0) : "memory");
  __builtin_unreachable();
}
#else
#  error "the freestanding exit stub does not support this architecture"
#endif
"#;

static LINK_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Returns the libraries named in a link request, for diagnostics.
fn libraries_text(inputs: &linker::LinkInputs<'_>) -> String {
    let mut names = Vec::new();
    let _ = inputs.objects.len();
    names.extend(inputs.source_libraries.iter().cloned());
    names.extend(inputs.libraries.iter().cloned());
    if names.is_empty() {
        "none (only the Seal runtime)".to_owned()
    } else {
        names.join(", ")
    }
}

/// Links an existing object file into an executable.
///
/// This is the entry point the project build graph uses: the frontend action
/// produces an object, the link action supplies the runtime bytes and native
/// libraries, and the output is replaced atomically on success.
pub(crate) fn link_object(request: &LinkRequest<'_>) -> Result<(), CodegenError> {
    run_linker(request)
}

/// Builds the path the linker writes before it replaces `output`.
///
/// The name lives in the destination directory and keeps its extension, so the
/// final rename stays on one filesystem and `cl`/`link` never append `.exe` on
/// their own. Toolchains that do append it when the name has no extension are
/// handled by giving Windows temporary paths an explicit one.
fn temporary_executable(output: &Path) -> Result<PathBuf, CodegenError> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let directory = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = output
        .file_name()
        .ok_or_else(|| CodegenError::artifact(output, "the output path has no file name"))?;
    let stem = output.file_stem().unwrap_or(file_name);
    let mut name = OsString::from(".");
    name.push(stem);
    name.push(format!(
        ".{}.{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    if let Some(extension) = output.extension() {
        name.push(".");
        name.push(extension);
    }
    let mut path = directory.join(name);
    if cfg!(windows) && path.extension().is_none() {
        path.set_extension("exe");
    }
    Ok(path)
}

struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct TempDirectory(PathBuf);

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

static TEMPORARY: AtomicU64 = AtomicU64::new(0);
