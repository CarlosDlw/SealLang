//! Writing object, assembly, and executable artifacts.
//!
//! Object and assembly output are written atomically through `seal-fs`. An
//! executable is linked from a temporary object in the output directory; the
//! temporary file is removed on every path, including linker failure.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use inkwell::module::Module;
use inkwell::targets::{FileType, TargetMachine};

use crate::error::CodegenError;
use crate::options::Emit;

/// Produces the requested artifact for `module`.
pub(crate) fn emit_artifact(
    module: &Module<'_>,
    machine: &TargetMachine,
    emit: Emit,
    output: &Path,
) -> Result<(), CodegenError> {
    match emit {
        Emit::Object => write_buffer(module, machine, FileType::Object, output),
        Emit::Assembly => write_buffer(module, machine, FileType::Assembly, output),
        Emit::Executable => link_executable(module, machine, output),
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

    let linker = linker_command();
    let result = Command::new(&linker)
        .arg(&object_path)
        .arg("-o")
        .arg(output)
        .output();
    match result {
        Ok(result) if result.status.success() => Ok(()),
        Ok(result) => Err(CodegenError::artifact(
            output,
            format!(
                "linker `{}` failed ({}): {}",
                linker.to_string_lossy(),
                result.status,
                String::from_utf8_lossy(&result.stderr).trim()
            ),
        )),
        Err(error) => Err(CodegenError::artifact(
            output,
            format!("cannot run linker `{}`: {error}", linker.to_string_lossy()),
        )),
    }
}

fn linker_command() -> OsString {
    std::env::var_os("SEAL_LINKER")
        .or_else(|| std::env::var_os("CC"))
        .unwrap_or_else(|| OsString::from("cc"))
}

struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
