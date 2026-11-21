//! Documentation generation errors.

use std::fmt;

/// A documentation generation failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DocError {
    /// A file could not be read or written.
    Io {
        /// The path involved.
        path: String,
        /// The operating-system message.
        message: String,
    },
    /// The output directory or an option is invalid.
    Invalid(String),
    /// JSON serialization failed.
    Json(String),
}

impl DocError {
    /// Returns the stable error code.
    pub fn code(&self) -> &'static str {
        "E8095"
    }
}

impl fmt::Display for DocError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => write!(formatter, "`{path}`: {message}"),
            Self::Invalid(message) => formatter.write_str(message),
            Self::Json(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for DocError {}

/// The result of a documentation operation.
pub type DocResult<T> = Result<T, DocError>;
