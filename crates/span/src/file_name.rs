//! Source file naming.

use std::fmt;
use std::path::{Path, PathBuf};

/// The name of a source file.
///
/// Real files are the ones loaded from disk. Virtual files are synthesized by the
/// compiler (command line, generated code) and never exist on disk.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileName {
    /// A file on disk, as passed to the compiler (not necessarily canonical).
    Real(PathBuf),
    /// A compiler-synthesized file such as `<command-line>`.
    Virtual(String),
}

impl FileName {
    /// Creates a real file name.
    #[inline]
    pub fn real(path: impl Into<PathBuf>) -> Self {
        Self::Real(path.into())
    }

    /// Creates a virtual file name. The displayed name is wrapped in angle
    /// brackets, so the conventional input is `command-line`, not
    /// `<command-line>`.
    pub fn virtual_name(name: impl Into<String>) -> Self {
        let name = name.into();
        if name.starts_with('<') && name.ends_with('>') {
            Self::Virtual(name)
        } else {
            Self::Virtual(format!("<{name}>"))
        }
    }

    /// Returns the on-disk path, if this is a real file.
    #[inline]
    pub fn as_real_path(&self) -> Option<&Path> {
        match self {
            Self::Real(path) => Some(path),
            Self::Virtual(_) => None,
        }
    }

    /// Returns `true` if this is a compiler-synthesized file.
    #[inline]
    pub fn is_virtual(&self) -> bool {
        matches!(self, Self::Virtual(_))
    }
}

impl fmt::Display for FileName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Real(path) => write!(f, "{}", path.display()),
            Self::Virtual(name) => f.write_str(name),
        }
    }
}

impl From<PathBuf> for FileName {
    fn from(path: PathBuf) -> Self {
        Self::Real(path)
    }
}

impl From<&Path> for FileName {
    fn from(path: &Path) -> Self {
        Self::Real(path.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::FileName;

    #[test]
    fn virtual_names_are_wrapped_once() {
        assert_eq!(
            FileName::virtual_name("command-line").to_string(),
            "<command-line>"
        );
        assert_eq!(
            FileName::virtual_name("<generated>").to_string(),
            "<generated>"
        );
        assert!(FileName::virtual_name("generated").is_virtual());
    }

    #[test]
    fn real_paths_display_as_given() {
        let name = FileName::real("examples/0001_main.sl");
        assert_eq!(name.to_string(), "examples/0001_main.sl");
        assert_eq!(
            name.as_real_path(),
            Some(std::path::Path::new("examples/0001_main.sl"))
        );
    }
}
