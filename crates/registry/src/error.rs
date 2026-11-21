//! Registry diagnostics with stable codes.

use std::fmt;

/// A registry failure with a stable diagnostic code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryError {
    code: &'static str,
    message: String,
}

impl RegistryError {
    /// Creates an error with an explicit code.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Invalid registry configuration (`E8020`).
    pub fn config(message: impl Into<String>) -> Self {
        Self::new("E8020", message)
    }

    /// A checksum or signed digest does not match (`E8021`).
    pub fn integrity(message: impl Into<String>) -> Self {
        Self::new("E8021", message)
    }

    /// A sparse index is missing or malformed (`E8025`).
    pub fn index(message: impl Into<String>) -> Self {
        Self::new("E8025", message)
    }

    /// Version resolution failed (`E8026`).
    pub fn resolve(message: impl Into<String>) -> Self {
        Self::new("E8026", message)
    }

    /// A package archive is malformed or unsafe (`E8027`).
    pub fn archive(message: impl Into<String>) -> Self {
        Self::new("E8027", message)
    }

    /// A vendor tree is inconsistent (`E8029`).
    pub fn vendor(message: impl Into<String>) -> Self {
        Self::new("E8029", message)
    }

    /// Audit policy rejected a package (`E8030`).
    pub fn audit(message: impl Into<String>) -> Self {
        Self::new("E8030", message)
    }

    /// SBOM generation failed (`E8031`).
    pub fn sbom(message: impl Into<String>) -> Self {
        Self::new("E8031", message)
    }

    /// Search or info failed (`E8032`).
    pub fn query(message: impl Into<String>) -> Self {
        Self::new("E8032", message)
    }

    /// A yanked version was requested explicitly (`E8033`).
    pub fn yanked(message: impl Into<String>) -> Self {
        Self::new("E8033", message)
    }

    /// A license is not allowed by policy (`E8034`).
    pub fn license(message: impl Into<String>) -> Self {
        Self::new("E8034", message)
    }

    /// A package source is not allowed by policy (`E8035`).
    pub fn source(message: impl Into<String>) -> Self {
        Self::new("E8035", message)
    }

    /// An advisory affects a locked package (`E8036`).
    pub fn advisory(message: impl Into<String>) -> Self {
        Self::new("E8036", message)
    }

    /// Required provenance is absent (`E8037`).
    pub fn provenance(message: impl Into<String>) -> Self {
        Self::new("E8037", message)
    }

    /// Registry metadata is internally inconsistent (`E8039`).
    pub fn metadata(message: impl Into<String>) -> Self {
        Self::new("E8039", message)
    }

    /// TUF metadata failed verification (`E8080`).
    pub fn tuf(message: impl Into<String>) -> Self {
        Self::new("E8080", message)
    }

    /// Authentication is missing or rejected (`E8084`).
    pub fn credentials(message: impl Into<String>) -> Self {
        Self::new("E8084", message)
    }

    /// An offline operation needs bytes that are not cached or vendored (`E8085`).
    pub fn offline(message: impl Into<String>) -> Self {
        Self::new("E8085", message)
    }

    /// A downloaded package disagrees with its index entry (`E8086`).
    pub fn package(message: impl Into<String>) -> Self {
        Self::new("E8086", message)
    }

    /// A registry cache entry is corrupt (`E8089`).
    pub fn cache(message: impl Into<String>) -> Self {
        Self::new("E8089", message)
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

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for RegistryError {}

/// The registry result type.
pub type RegistryResult<T> = Result<T, RegistryError>;
