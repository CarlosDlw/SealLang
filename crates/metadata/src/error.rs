//! Errors produced while reading or writing a `.sealmono` artifact.

use std::fmt;

use seal_error_codes::E8070;

/// A malformed, corrupted, or unsupported `.sealmono` artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataError {
    message: String,
}

impl MetadataError {
    /// Creates an error with a detailed message.
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Returns the diagnostic code assigned to this failure.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        E8070.code()
    }

    /// Returns the human-readable message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for MetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code(), self.message)
    }
}

impl std::error::Error for MetadataError {}
