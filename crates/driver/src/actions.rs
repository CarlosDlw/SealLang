//! Informational actions: they print and exit without compiling anything.

use std::path::Path;
use std::process::ExitCode;

use seal_target::host_triple;
use seal_version::LanguageVersion;

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

/// Prints the compiler version, language version, and host triple.
pub(crate) fn version() -> Result<ExitCode, String> {
    println!("sealc {}", env!("CARGO_PKG_VERSION"));
    println!("language: {}", LanguageVersion::CURRENT);
    println!("host: {}", host_triple());
    Ok(ExitCode::SUCCESS)
}

/// Writes the embedded runtime archive to `path`.
pub(crate) fn emit_runtime(path: &Path) -> Result<ExitCode, String> {
    seal_fs::write_atomic(path, seal_codegen::runtime_archive())
        .map_err(|error| format!("cannot write runtime archive `{}`: {error}", path.display()))?;
    Ok(ExitCode::SUCCESS)
}
