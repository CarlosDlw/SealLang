//! Standard-library diagnostics.

use std::fmt;

/// A standard-library failure with a stable diagnostic code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StdlibError {
    code: &'static str,
    message: String,
}

impl StdlibError {
    /// Creates an error with an explicit code.
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// The selected `SEAL_STDLIB` is missing or invalid.
    pub(crate) fn locate(message: impl Into<String>) -> Self {
        Self::new("E8133", message)
    }

    /// The descriptor is malformed or inconsistent.
    pub(crate) fn descriptor(message: impl Into<String>) -> Self {
        Self::new("E8135", message)
    }

    /// The standard-library digest is missing or does not match its contents.
    pub(crate) fn digest(message: impl Into<String>) -> Self {
        Self::new("E8135", message)
    }

    /// A package directory, manifest, source file, or path is invalid.
    pub(crate) fn package(message: impl Into<String>) -> Self {
        Self::new("E8139", message)
    }

    /// The descriptor targets a different language version.
    pub(crate) fn language(message: impl Into<String>) -> Self {
        Self::new("E8134", message)
    }

    /// The package list, internal dependency declarations, or graph is invalid.
    pub(crate) fn dependencies(message: impl Into<String>) -> Self {
        Self::new("E8137", message)
    }

    /// Source contents do not match the descriptor digest.
    pub(crate) fn verify(message: impl Into<String>) -> Self {
        Self::new("E8140", message)
    }

    /// Returns the diagnostic code.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Returns the human-readable message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for StdlibError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for StdlibError {}

/// The result type for standard-library operations.
pub type StdlibResult<T> = Result<T, StdlibError>;
