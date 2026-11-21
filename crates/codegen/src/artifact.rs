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

/// Produces the requested artifact for `module`.
pub(crate) fn emit_artifact(
    module: &Module<'_>,
    machine: &TargetMachine,
    options: &CodegenOptions,
    runtime: &[u8],
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
    runtime: &[u8],
    source_libraries: &[String],
) -> Result<(), CodegenError> {
    link_module(module, machine, options, runtime, source_libraries, false)
}

fn link_cdylib(
    module: &Module<'_>,
    machine: &TargetMachine,
    options: &CodegenOptions,
    runtime: &[u8],
    source_libraries: &[String],
) -> Result<(), CodegenError> {
    link_module(module, machine, options, runtime, source_libraries, true)
}

fn link_staticlib(
    module: &Module<'_>,
    machine: &TargetMachine,
    options: &CodegenOptions,
    runtime: &[u8],
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
    runtime: &[u8],
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
        runtime: Some(runtime),
        output,
        shared,
        source_libraries,
        libraries: &options.libraries,
        search_paths: &options.search_paths,
        link_args: &options.link_args,
        linker: None,
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
    runtime: &[u8],
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
    seal_fs::write_atomic(&runtime_path, runtime).map_err(|error| {
        CodegenError::artifact(
            &runtime_path,
            format!("cannot write the temporary runtime archive: {error}"),
        )
    })?;

    if cfg!(windows) {
        let program = std::env::var_os("SEAL_AR").unwrap_or_else(|| OsString::from("lib"));
        let mut command = Command::new(&program);
        command.arg("/NOLOGO").arg(prefixed("/OUT:", &temporary));
        for object in objects {
            command.arg(object);
        }
        command.arg(&runtime_path);
        run_tool(command, &program, output)?;
    } else {
        let combined = work.join("combined.o");
        let compiler = std::env::var_os("CC").unwrap_or_else(|| OsString::from("cc"));
        let mut partial = Command::new(&compiler);
        partial.arg("-r").arg("-o").arg(&combined);
        for object in objects {
            partial.arg(object);
        }
        partial.arg(&runtime_path);
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
    let inputs = linker::LinkInputs {
        objects: request
            .objects
            .iter()
            .map(|path| path.to_path_buf())
            .collect(),
        runtime: &runtime_path,
        output: &temporary_path,
        shared: request.shared,
        source_libraries: request.source_libraries,
        libraries: request.libraries,
        search_paths: request.search_paths,
        link_args: request.link_args,
    };
    // Validate before any temporary file is written: a bad `-l` or `-L` must
    // not leave a runtime archive or a partial executable behind.
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
    let program = request
        .linker
        .map(std::ffi::OsStr::to_owned)
        .unwrap_or_else(linker::default_program);
    let invocation = linker::invocation(program, &inputs);
    let result = Command::new(invocation.program())
        .args(invocation.arguments())
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
        Err(error) => Err(CodegenError::artifact(
            output,
            format!(
                "cannot run linker `{}`: {error}\ncommand: {}\nlibraries: {}",
                invocation.program().to_string_lossy(),
                invocation.command_line(),
                libraries_text(&inputs),
            ),
        )),
    }
}

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
