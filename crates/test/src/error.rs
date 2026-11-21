//! Typed failures from discovery and the runner.

use std::fmt;

/// A test-runner failure that no test can represent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestError {
    /// A harness executable could not be executed (`E8101`).
    Harness {
        /// The executable that failed.
        path: String,
        /// The reason, including the operating-system error.
        detail: String,
    },
    /// A runner option is invalid (`E8102`).
    Option {
        /// The option and the reason it is invalid.
        detail: String,
    },
    /// Results could not be written (`E8108`).
    Report {
        /// The destination and the reason.
        detail: String,
    },
}

impl TestError {
    /// Returns the stable diagnostic code.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Harness { .. } => "E8101",
            Self::Option { .. } => "E8102",
            Self::Report { .. } => "E8108",
        }
    }

    /// Creates a harness failure.
    pub fn harness(path: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::Harness {
            path: path.into(),
            detail: detail.into(),
        }
    }

    /// Creates an invalid-option failure.
    pub fn option(detail: impl Into<String>) -> Self {
        Self::Option {
            detail: detail.into(),
        }
    }

    /// Creates a report-writing failure.
    pub fn report(detail: impl Into<String>) -> Self {
        Self::Report {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for TestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Harness { path, detail } => {
                write!(f, "cannot run the test harness `{path}`: {detail}")
            }
            Self::Option { detail } => f.write_str(detail),
            Self::Report { detail } => f.write_str(detail),
        }
    }
}

impl std::error::Error for TestError {}
