//! Compile-plan failures.

use std::fmt;

/// A plan failure with the diagnostic code the driver reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanError {
    code: &'static str,
    message: String,
}

impl PlanError {
    /// Creates a structural plan failure (`E8010`).
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "E8010",
            message: message.into(),
        }
    }

    /// Creates a plan-file failure (`E8001`).
    pub fn schema(message: impl Into<String>) -> Self {
        Self {
            code: "E8001",
            message: message.into(),
        }
    }

    /// Creates an I/O failure (`E8001`).
    pub fn io(message: impl Into<String>) -> Self {
        Self::schema(message)
    }

    /// Creates a schema-version failure (`E8001`).
    pub fn version(found: u32) -> Self {
        Self::schema(format!(
            "the compile plan uses schema version {found}, but this compiler understands version {}",
            crate::FORMAT_VERSION
        ))
    }

    /// Returns the stable diagnostic code.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Returns the human-readable message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for PlanError {}

/// The result type for plan operations.
pub type PlanResult<T> = Result<T, PlanError>;
