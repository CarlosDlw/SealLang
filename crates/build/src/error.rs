//! Build failures.

use std::fmt;

use seal_cas::CasError;
use seal_project::ProjectError;
use seal_sandbox::SandboxError;

/// A failure while constructing or executing the action graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildError {
    code: &'static str,
    message: String,
}

impl BuildError {
    /// Creates an error with a stable code.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// A cache or content-addressed store failure (`E8040`).
    pub fn cache(message: impl Into<String>) -> Self {
        Self::new("E8040", message)
    }

    /// A sandbox or input-visibility failure (`E8041`).
    pub fn input(message: impl Into<String>) -> Self {
        Self::new("E8041", message)
    }

    /// A generator contract failure (`E8042`).
    pub fn generator(message: impl Into<String>) -> Self {
        Self::new("E8042", message)
    }

    /// A native dependency failure (`E8043`).
    pub fn native(message: impl Into<String>) -> Self {
        Self::new("E8043", message)
    }

    /// A sandbox timeout (`E8044`).
    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new("E8044", message)
    }

    /// An action graph or action execution failure (`E8045`).
    pub fn action(message: impl Into<String>) -> Self {
        Self::new("E8045", message)
    }

    /// A task failure (`E8046`).
    pub fn task(message: impl Into<String>) -> Self {
        Self::new("E8046", message)
    }

    /// Two outputs claim the same path (`E8047`).
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new("E8047", message)
    }

    /// Returns the stable diagnostic code.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Returns the message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for BuildError {}

impl From<CasError> for BuildError {
    fn from(error: CasError) -> Self {
        Self::new(error.code(), error.to_string())
    }
}

impl From<SandboxError> for BuildError {
    fn from(error: SandboxError) -> Self {
        Self::new(error.code(), error.to_string())
    }
}

impl From<ProjectError> for BuildError {
    fn from(error: ProjectError) -> Self {
        Self::new(error.code(), error.message().to_owned())
    }
}

/// The result type for build operations.
pub type BuildResult<T> = Result<T, BuildError>;
