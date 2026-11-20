//! Linker command construction for the supported C toolchains.
//!
//! Unix C compilers and MinGW use `-o`; MSVC's `cl`/`clang-cl` use `/Fe:` and
//! pass linker-only options after `/link`; the standalone MSVC linker uses
//! `/OUT:`. The flavor is derived from the program name, so users can select a
//! toolchain through `SEAL_LINKER` or `CC` without an extra dialect flag.
//!
//! Arguments are always passed as an argument vector, never through a shell.
//! Library names are validated before any temporary file is written, so a bad
//! `-l` or `-L` cannot leave an artifact or a partial link behind.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::error::CodegenError;

/// One native link request.
pub(crate) struct LinkInputs<'a> {
    /// The generated object file.
    pub(crate) object: &'a Path,
    /// The Seal runtime archive.
    pub(crate) runtime: &'a Path,
    /// The executable written by the linker.
    pub(crate) output: &'a Path,
    /// Libraries declared by `link "..."` in the source, in source order.
    pub(crate) source_libraries: &'a [String],
    /// Libraries requested with `-l`, in request order.
    pub(crate) libraries: &'a [String],
    /// Directories added with `-L`, in request order.
    pub(crate) search_paths: &'a [PathBuf],
    /// Arguments passed through with `--link-arg`, in request order.
    pub(crate) link_args: &'a [OsString],
}

impl LinkInputs<'_> {
    /// Returns every library in the order it reaches the linker.
    fn libraries_in_order(&self) -> impl Iterator<Item = &String> {
        self.source_libraries.iter().chain(self.libraries)
    }
}

/// How a linker expects its output path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flavor {
    /// `cc`, `gcc`, `clang`, and MinGW: `-o <path>`.
    Gnu,
    /// MSVC `cl` and `clang-cl`: `/Fe:<path>` and `/link`.
    MsvcCompiler,
    /// The standalone MSVC linker: `/OUT:<path>`.
    MsvcLinker,
}

/// A linker program and its fully constructed argument vector.
pub(crate) struct Invocation {
    program: OsString,
    arguments: Vec<OsString>,
}

impl Invocation {
    /// Returns the program to execute.
    pub(crate) fn program(&self) -> &OsStr {
        &self.program
    }

    /// Returns the arguments, excluding the program name.
    pub(crate) fn arguments(&self) -> &[OsString] {
        &self.arguments
    }

    /// Returns a readable command line for diagnostics.
    ///
    /// The rendering is only for humans: the linker is always executed with
    /// the argument vector, never with this string.
    pub(crate) fn command_line(&self) -> String {
        let mut text = self.program.to_string_lossy().into_owned();
        for argument in &self.arguments {
            text.push(' ');
            text.push_str(&format!("{argument:?}"));
        }
        text
    }
}

/// Returns the linker selected by the environment or the platform default.
pub(crate) fn default_program() -> OsString {
    std::env::var_os("SEAL_LINKER")
        .or_else(|| std::env::var_os("CC"))
        .unwrap_or_else(|| OsString::from(if cfg!(windows) { "cl" } else { "cc" }))
}

/// Rejects a library name or search path the linker cannot receive safely.
///
/// A name is passed as one argument, so it may not smuggle a path or another
/// flag into the command line. The rules are platform-independent: a name that
/// would be ambiguous on one supported toolchain is rejected everywhere.
pub(crate) fn validate(inputs: &LinkInputs<'_>) -> Result<(), CodegenError> {
    for library in inputs.libraries_in_order() {
        validate_library_name(library)?;
    }
    for path in inputs.search_paths {
        if !path.exists() {
            return Err(CodegenError::link_input(format!(
                "the link search path `{}` does not exist",
                path.display()
            )));
        }
        if !path.is_dir() {
            return Err(CodegenError::link_input(format!(
                "the link search path `{}` is not a directory",
                path.display()
            )));
        }
    }
    Ok(())
}

fn validate_library_name(name: &str) -> Result<(), CodegenError> {
    if name.is_empty() {
        return Err(CodegenError::link_input(
            "a library name cannot be empty".to_owned(),
        ));
    }
    if name.contains('/') || name.contains('\\') {
        return Err(CodegenError::link_input(format!(
            "the library name `{name}` cannot contain a path separator"
        )));
    }
    if name.chars().any(char::is_whitespace) {
        return Err(CodegenError::link_input(format!(
            "the library name `{name}` cannot contain whitespace"
        )));
    }
    if name.chars().any(char::is_control) {
        return Err(CodegenError::link_input(format!(
            "the library name `{name}` cannot contain control characters"
        )));
    }
    Ok(())
}

/// Builds the invocation that links `inputs` into its output.
///
/// The order is object, source libraries, command-line libraries, extra
/// arguments, then the runtime archive. On MSVC the runtime is the last input
/// after `/link`, keeping every linker-only option in one region.
pub(crate) fn invocation(program: OsString, inputs: &LinkInputs<'_>) -> Invocation {
    let mut arguments = Vec::new();
    match flavor(&program) {
        Flavor::Gnu => {
            arguments.push(inputs.object.as_os_str().to_owned());
            for path in inputs.search_paths {
                arguments.push(OsString::from("-L"));
                arguments.push(path.as_os_str().to_owned());
            }
            for library in inputs.libraries_in_order() {
                arguments.push(OsString::from(format!("-l{library}")));
            }
            arguments.extend(inputs.link_args.iter().cloned());
            arguments.push(inputs.runtime.as_os_str().to_owned());
            arguments.push(OsString::from("-o"));
            arguments.push(inputs.output.as_os_str().to_owned());
        }
        Flavor::MsvcCompiler => {
            arguments.push(OsString::from("/nologo"));
            arguments.push(inputs.object.as_os_str().to_owned());
            arguments.push(prefixed("/Fe:", inputs.output));
            arguments.push(OsString::from("/link"));
            for path in inputs.search_paths {
                arguments.push(prefixed("/LIBPATH:", path));
            }
            for library in inputs.libraries_in_order() {
                arguments.push(msvc_library(library));
            }
            arguments.extend(inputs.link_args.iter().cloned());
            arguments.push(inputs.runtime.as_os_str().to_owned());
        }
        Flavor::MsvcLinker => {
            arguments.push(OsString::from("/NOLOGO"));
            arguments.push(inputs.object.as_os_str().to_owned());
            arguments.push(prefixed("/OUT:", inputs.output));
            for path in inputs.search_paths {
                arguments.push(prefixed("/LIBPATH:", path));
            }
            for library in inputs.libraries_in_order() {
                arguments.push(msvc_library(library));
            }
            arguments.extend(inputs.link_args.iter().cloned());
            arguments.push(inputs.runtime.as_os_str().to_owned());
        }
    }
    Invocation { program, arguments }
}

fn flavor(program: &OsStr) -> Flavor {
    let name = Path::new(program)
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or_default();
    if name.eq_ignore_ascii_case("cl") || name.eq_ignore_ascii_case("clang-cl") {
        Flavor::MsvcCompiler
    } else if name.eq_ignore_ascii_case("link") {
        Flavor::MsvcLinker
    } else {
        Flavor::Gnu
    }
}

fn prefixed(prefix: &str, path: &Path) -> OsString {
    let mut argument = OsString::from(prefix);
    argument.push(path);
    argument
}

/// Returns the MSVC library input for `name`.
fn msvc_library(name: &str) -> OsString {
    if name.ends_with(".lib") {
        OsString::from(name)
    } else {
        OsString::from(format!("{name}.lib"))
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::{OsStr, OsString};
    use std::path::{Path, PathBuf};

    use crate::error::CodegenError;

    use super::{Flavor, LinkInputs, flavor, invocation, validate};

    fn arguments(program: &str, inputs: &LinkInputs<'_>) -> Vec<String> {
        invocation(OsString::from(program), inputs)
            .arguments()
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }

    fn inputs<'a>(
        source_libraries: &'a [String],
        libraries: &'a [String],
        search_paths: &'a [PathBuf],
        link_args: &'a [OsString],
    ) -> LinkInputs<'a> {
        LinkInputs {
            object: Path::new("app.o"),
            runtime: Path::new("seal_rt.lib"),
            output: Path::new("app.exe"),
            source_libraries,
            libraries,
            search_paths,
            link_args,
        }
    }

    #[test]
    fn gnu_places_the_runtime_last_and_search_paths_before_libraries() {
        let source = vec!["fixture".to_owned()];
        let cli = vec!["m".to_owned(), "pthread".to_owned()];
        let paths = vec![PathBuf::from("/tmp/lib")];
        let extras = vec![OsString::from("extra.o")];
        let inputs = inputs(&source, &cli, &paths, &extras);
        assert_eq!(
            arguments("cc", &inputs),
            vec![
                "app.o",
                "-L",
                "/tmp/lib",
                "-lfixture",
                "-lm",
                "-lpthread",
                "extra.o",
                "seal_rt.lib",
                "-o",
                "app.exe",
            ]
        );
    }

    #[test]
    fn msvc_compilers_use_fe_link_and_libpath() {
        let source = vec!["fixture".to_owned()];
        let cli = vec!["user32".to_owned()];
        let paths = vec![PathBuf::from(r"C:\libs")];
        let extras = vec![OsString::from("/DEBUG")];
        let inputs = inputs(&source, &cli, &paths, &extras);
        for program in ["cl", "clang-cl"] {
            assert_eq!(
                arguments(program, &inputs),
                vec![
                    "/nologo",
                    "app.o",
                    "/Fe:app.exe",
                    "/link",
                    r"/LIBPATH:C:\libs",
                    "fixture.lib",
                    "user32.lib",
                    "/DEBUG",
                    "seal_rt.lib",
                ]
            );
        }
    }

    #[test]
    fn the_standalone_msvc_linker_uses_out_and_libpath() {
        let source = vec!["fixture".to_owned()];
        let paths = vec![PathBuf::from(r"C:\libs")];
        let inputs = inputs(&source, &[], &paths, &[]);
        assert_eq!(
            arguments("link", &inputs),
            vec![
                "/NOLOGO",
                "app.o",
                "/OUT:app.exe",
                r"/LIBPATH:C:\libs",
                "fixture.lib",
                "seal_rt.lib",
            ]
        );
    }

    #[test]
    fn two_source_libraries_keep_their_order_before_cli_libraries() {
        let source = vec!["first".to_owned(), "second".to_owned()];
        let cli = vec!["third".to_owned()];
        let inputs = inputs(&source, &cli, &[], &[]);
        assert_eq!(
            arguments("cc", &inputs),
            vec![
                "app.o",
                "-lfirst",
                "-lsecond",
                "-lthird",
                "seal_rt.lib",
                "-o",
                "app.exe",
            ]
        );
    }

    #[test]
    fn an_empty_or_path_like_library_name_is_invalid() {
        for name in ["", "a/b", r"a\b", "a b", "a\tb"] {
            let libraries = vec![name.to_owned()];
            let inputs = inputs(&[], &libraries, &[], &[]);
            assert!(
                matches!(validate(&inputs), Err(CodegenError::LinkInput { .. })),
                "`{name}` must be rejected"
            );
        }
    }

    #[test]
    fn an_ordinary_library_name_is_valid() {
        let libraries = vec!["sealfixture".to_owned(), "sqlite3".to_owned()];
        let inputs = inputs(&[], &libraries, &[], &[]);
        assert!(validate(&inputs).is_ok());
    }

    #[test]
    fn a_missing_search_path_is_invalid() {
        let missing = std::env::temp_dir().join(format!(
            "seal-link-path-that-does-not-exist-{}",
            std::process::id()
        ));
        let paths = vec![missing];
        let inputs = inputs(&[], &[], &paths, &[]);
        assert!(matches!(
            validate(&inputs),
            Err(CodegenError::LinkInput { .. })
        ));
    }

    #[test]
    fn flavor_detection_is_case_insensitive_and_ignores_exe() {
        assert_eq!(flavor(OsStr::new("CL.EXE")), Flavor::MsvcCompiler);
        assert_eq!(flavor(OsStr::new("Clang-CL.exe")), Flavor::MsvcCompiler);
        assert_eq!(flavor(OsStr::new("LINK.EXE")), Flavor::MsvcLinker);
        assert_eq!(flavor(OsStr::new("cc")), Flavor::Gnu);
        assert_eq!(
            flavor(OsStr::new("x86_64-w64-mingw32-gcc.exe")),
            Flavor::Gnu
        );
    }
}
