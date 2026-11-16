//! Informational actions: they print and exit without compiling anything.

use std::path::Path;
use std::process::ExitCode;

/// Prints the explanation for `code` and reports success.
pub(crate) fn explain(code: &str) -> Result<ExitCode, String> {
    match seal_error_codes::explain(code) {
        Some(explanation) => {
            print!("{explanation}");
            Ok(ExitCode::SUCCESS)
        }
        None => Err(format!("unknown error code `{code}`")),
    }
}

/// Writes the embedded runtime archive to `path`.
pub(crate) fn emit_runtime(path: &Path) -> Result<ExitCode, String> {
    seal_fs::write_atomic(path, seal_codegen::runtime_archive())
        .map_err(|error| format!("cannot write runtime archive `{}`: {error}", path.display()))?;
    Ok(ExitCode::SUCCESS)
}
