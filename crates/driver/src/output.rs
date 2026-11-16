//! Output path selection for artifacts.

use std::path::{Path, PathBuf};

use seal_codegen::Emit;

use crate::cli::Cli;

/// Returns the path an artifact is written to.
pub(crate) fn artifact_path(cli: &Cli, input: &Path, emit: Emit) -> PathBuf {
    let requested = cli
        .output
        .path
        .clone()
        .unwrap_or_else(|| default_output(input, emit));
    with_executable_extension(requested, emit, cfg!(windows))
}

/// Returns the output path used when `-o` is absent.
fn default_output(input: &Path, emit: Emit) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "seal-output".to_owned());
    let name = match emit {
        Emit::Object => format!("{stem}.o"),
        Emit::Assembly => format!("{stem}.s"),
        Emit::Executable => stem,
    };
    input.with_file_name(name)
}

/// Adds the Windows executable extension when the caller omitted one.
fn with_executable_extension(path: PathBuf, emit: Emit, windows: bool) -> PathBuf {
    if windows && emit == Emit::Executable && path.extension().is_none() {
        path.with_extension("exe")
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use seal_codegen::Emit;

    use super::{default_output, with_executable_extension};

    #[test]
    fn a_windows_executable_without_an_extension_gets_exe() {
        assert_eq!(
            with_executable_extension(PathBuf::from("program"), Emit::Executable, true),
            PathBuf::from("program.exe")
        );
    }

    #[test]
    fn explicit_extensions_and_non_executables_are_preserved() {
        assert_eq!(
            with_executable_extension(PathBuf::from("program.bin"), Emit::Executable, true),
            PathBuf::from("program.bin")
        );
        assert_eq!(
            with_executable_extension(PathBuf::from("program.o"), Emit::Object, true),
            PathBuf::from("program.o")
        );
        assert_eq!(
            with_executable_extension(PathBuf::from("program"), Emit::Executable, false),
            PathBuf::from("program")
        );
    }

    #[test]
    fn default_outputs_live_next_to_the_input() {
        assert_eq!(
            default_output(Path::new("src/main.sl"), Emit::Executable),
            PathBuf::from("src/main")
        );
        assert_eq!(
            default_output(Path::new("src/main.sl"), Emit::Object),
            PathBuf::from("src/main.o")
        );
        assert_eq!(
            default_output(Path::new("src/main.sl"), Emit::Assembly),
            PathBuf::from("src/main.s")
        );
    }
}
