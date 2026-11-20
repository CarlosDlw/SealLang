//! Manifest diagnostics.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A position in the manifest, one-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    /// One-based line number.
    pub line: usize,
    /// One-based column number.
    pub column: usize,
}

impl Location {
    /// Creates a location.
    pub const fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }

    /// Computes a location from a byte offset in `source`.
    pub fn from_offset(source: &str, offset: usize) -> Self {
        let offset = offset.min(source.len());
        let prefix = &source[..offset];
        let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let column = prefix
            .rsplit_once('\n')
            .map_or(prefix.len(), |(_, tail)| tail.len())
            + 1;
        Self { line, column }
    }
}

/// A fatal manifest diagnostic (`E8005` or `E8011`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestError {
    code: &'static str,
    message: String,
    location: Option<Location>,
}

impl ManifestError {
    /// Creates an invalid manifest failure (`E8005`).
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "E8005",
            message: message.into(),
            location: None,
        }
    }

    /// Creates a manifest I/O failure (`E8011`).
    pub fn io(message: impl Into<String>) -> Self {
        Self {
            code: "E8011",
            message: message.into(),
            location: None,
        }
    }

    /// Attaches a source location.
    pub fn at(mut self, location: Option<Location>) -> Self {
        self.location = location;
        self
    }

    /// Returns the diagnostic code.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Returns the message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the optional source location.
    pub fn location(&self) -> Option<Location> {
        self.location
    }
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.location {
            Some(location) => write!(
                f,
                "{}:{}:{}: error: {}",
                self.code, location.line, location.column, self.message
            ),
            None => write!(f, "{}: error: {}", self.code, self.message),
        }
    }
}

impl std::error::Error for ManifestError {}

/// A non-fatal manifest diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestWarning {
    /// The warning message.
    pub message: String,
    /// The key's source location.
    pub location: Option<Location>,
    /// A suggested replacement key.
    pub suggestion: Option<String>,
}

/// The result type for manifest operations.
pub type ManifestResult<T> = Result<T, ManifestError>;
