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
use crate::options::Emit;

/// Produces the requested artifact for `module`.
pub(crate) fn emit_artifact(
    module: &Module<'_>,
    machine: &TargetMachine,
    emit: Emit,
    output: &Path,
    runtime: &[u8],
) -> Result<(), CodegenError> {
    match emit {
        Emit::Object => write_buffer(module, machine, FileType::Object, output),
        Emit::Assembly => write_buffer(module, machine, FileType::Assembly, output),
        Emit::Executable => link_executable(module, machine, output, runtime),
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
    output: &Path,
    runtime: &[u8],
) -> Result<(), CodegenError> {
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

    let runtime_path = directory.join(format!(
        ".{}.{}.seal_rt{}",
        file_name.to_string_lossy(),
        std::process::id(),
        if cfg!(windows) { ".lib" } else { ".a" }
    ));
    let _runtime_guard = TempFile(runtime_path.clone());
    seal_fs::write_atomic(&runtime_path, runtime).map_err(|error| {
        CodegenError::artifact(
            &runtime_path,
            format!("cannot write the temporary runtime archive: {error}"),
        )
    })?;

    let temporary_path = temporary_executable(output)?;
    let _temporary_guard = TempFile(temporary_path.clone());
    let invocation = linker::invocation(
        linker::default_program(),
        &object_path,
        &runtime_path,
        &temporary_path,
    );
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
                "linker `{}` failed ({}): {}",
                invocation.program().to_string_lossy(),
                result.status,
                String::from_utf8_lossy(&result.stderr).trim()
            ),
        )),
        Err(error) => Err(CodegenError::artifact(
            output,
            format!(
                "cannot run linker `{}`: {error}",
                invocation.program().to_string_lossy()
            ),
        )),
    }
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
