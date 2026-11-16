//! Linker command construction for the supported C toolchains.
//!
//! Unix C compilers and MinGW use `-o`; MSVC's `cl`/`clang-cl` use `/Fe:`, and
//! the standalone MSVC linker uses `/OUT:`. The flavor is derived from the
//! program name, so users can select a toolchain through `SEAL_LINKER` or `CC`
//! without an extra dialect flag.

use std::ffi::{OsStr, OsString};
use std::path::Path;

/// How a linker expects its output path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flavor {
    /// `cc`, `gcc`, `clang`, and MinGW: `-o <path>`.
    Gnu,
    /// MSVC `cl` and `clang-cl`: `/Fe:<path>`.
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
}

/// Returns the linker selected by the environment or the platform default.
pub(crate) fn default_program() -> OsString {
    std::env::var_os("SEAL_LINKER")
        .or_else(|| std::env::var_os("CC"))
        .unwrap_or_else(|| OsString::from(if cfg!(windows) { "cl" } else { "cc" }))
}

/// Builds the invocation that links `object` and `runtime` into `output`.
pub(crate) fn invocation(
    program: OsString,
    object: &Path,
    runtime: &Path,
    output: &Path,
) -> Invocation {
    let mut arguments = Vec::with_capacity(5);
    match flavor(&program) {
        Flavor::Gnu => {
            arguments.push(object.as_os_str().to_owned());
            arguments.push(runtime.as_os_str().to_owned());
            arguments.push(OsString::from("-o"));
            arguments.push(output.as_os_str().to_owned());
        }
        Flavor::MsvcCompiler => {
            arguments.push(OsString::from("/nologo"));
            arguments.push(object.as_os_str().to_owned());
            arguments.push(runtime.as_os_str().to_owned());
            arguments.push(prefixed("/Fe:", output));
        }
        Flavor::MsvcLinker => {
            arguments.push(OsString::from("/NOLOGO"));
            arguments.push(object.as_os_str().to_owned());
            arguments.push(runtime.as_os_str().to_owned());
            arguments.push(prefixed("/OUT:", output));
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

#[cfg(test)]
mod tests {
    use std::ffi::{OsStr, OsString};
    use std::path::Path;

    use super::{Flavor, flavor, invocation};

    fn arguments(program: &str) -> Vec<String> {
        invocation(
            OsString::from(program),
            Path::new("app.o"),
            Path::new("seal_rt.lib"),
            Path::new("app.exe"),
        )
        .arguments()
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect()
    }

    #[test]
    fn gnu_style_places_objects_before_the_output_flag() {
        assert_eq!(
            arguments("cc"),
            vec!["app.o", "seal_rt.lib", "-o", "app.exe"]
        );
    }

    #[test]
    fn msvc_compilers_use_fe_and_suppress_the_banner() {
        assert_eq!(
            arguments("cl"),
            vec!["/nologo", "app.o", "seal_rt.lib", "/Fe:app.exe"]
        );
        assert_eq!(
            arguments("clang-cl"),
            vec!["/nologo", "app.o", "seal_rt.lib", "/Fe:app.exe"]
        );
    }

    #[test]
    fn the_standalone_msvc_linker_uses_out() {
        assert_eq!(
            arguments("link"),
            vec!["/NOLOGO", "app.o", "seal_rt.lib", "/OUT:app.exe"]
        );
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
