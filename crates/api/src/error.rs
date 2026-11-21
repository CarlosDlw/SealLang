//! API snapshot diagnostics with stable codes.

use std::fmt;

/// A failure while reading, hashing, or comparing API snapshots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiError {
    code: &'static str,
    message: String,
}

impl ApiError {
    /// Creates an error with an explicit code.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// The snapshot is malformed or uses an unknown schema (`E8090`).
    pub fn snapshot(message: impl Into<String>) -> Self {
        Self::new("E8090", message)
    }

    /// The new snapshot breaks compatibility (`E8091`).
    pub fn breaking(message: impl Into<String>) -> Self {
        Self::new("E8091", message)
    }

    /// A breaking change was published without a major bump (`E8092`).
    pub fn version(message: impl Into<String>) -> Self {
        Self::new("E8092", message)
    }

    /// Returns the diagnostic code.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Returns the message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

/// The API result type.
pub type ApiResult<T> = Result<T, ApiError>;
