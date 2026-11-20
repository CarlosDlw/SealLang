//! Project diagnostics.

use std::fmt;

/// A project failure with a stable diagnostic code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectError {
    code: &'static str,
    message: String,
}

impl ProjectError {
    /// Creates a project error with the given code.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// A missing or unreadable project file (`E8011`).
    pub fn io(message: impl Into<String>) -> Self {
        Self::new("E8011", message)
    }

    /// A manifest failure (`E8005`).
    pub fn manifest(message: impl Into<String>) -> Self {
        Self::new("E8005", message)
    }

    /// An unsupported dependency (`E8002`).
    pub fn dependency(message: impl Into<String>) -> Self {
        Self::new("E8002", message)
    }

    /// An invalid target (`E8003`).
    pub fn target(message: impl Into<String>) -> Self {
        Self::new("E8003", message)
    }

    /// A target entry function is missing (`E2087`).
    pub fn entry(message: impl Into<String>) -> Self {
        Self::new("E2087", message)
    }

    /// A source glob with no matches (`E8004`).
    pub fn source(message: impl Into<String>) -> Self {
        Self::new("E8004", message)
    }

    /// A dependency cycle (`E8006`).
    pub fn cycle(message: impl Into<String>) -> Self {
        Self::new("E8006", message)
    }

    /// An invalid, unknown, or conflicting feature (`E8007`).
    pub fn feature(message: impl Into<String>) -> Self {
        Self::new("E8007", message)
    }

    /// A path dependency that leaves the project (`E8021`).
    pub fn path_escape(message: impl Into<String>) -> Self {
        Self::new("E8021", message)
    }

    /// A missing path dependency (`E8022`).
    pub fn path_dependency(message: impl Into<String>) -> Self {
        Self::new("E8022", message)
    }

    /// An invalid dependency declaration (`E8024`).
    pub fn dependency_declaration(message: impl Into<String>) -> Self {
        Self::new("E8024", message)
    }

    /// An incompatible dependency version (`E8023`).
    pub fn version(message: impl Into<String>) -> Self {
        Self::new("E8023", message)
    }

    /// A lockfile failure (`E8008`).
    pub fn lockfile(message: impl Into<String>) -> Self {
        Self::new("E8008", message)
    }

    /// An invalid profile (`E8009`).
    pub fn profile(message: impl Into<String>) -> Self {
        Self::new("E8009", message)
    }

    /// An invalid build plan (`E8010`).
    pub fn plan(message: impl Into<String>) -> Self {
        Self::new("E8010", message)
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

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ProjectError {}

/// The result type for project operations.
pub type ProjectResult<T> = Result<T, ProjectError>;
