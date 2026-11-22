//! Server-side failures mapped to gRPC statuses.

use std::fmt;

use tonic::Status;

/// A remote server failure with a stable code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerError {
    code: &'static str,
    status: tonic::Code,
    message: String,
}

impl ServerError {
    /// Creates an invalid-argument error (`E8120`).
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("E8120", tonic::Code::InvalidArgument, message)
    }

    /// Creates an authentication error (`E8123`).
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new("E8123", tonic::Code::Unauthenticated, message)
    }

    /// Creates a missing-blob error (`E8124`).
    pub fn missing(message: impl Into<String>) -> Self {
        Self::new("E8124", tonic::Code::NotFound, message)
    }

    /// Creates an integrity error (`E8122`).
    pub fn integrity(message: impl Into<String>) -> Self {
        Self::new("E8122", tonic::Code::InvalidArgument, message)
    }

    /// Creates an unsupported-capability error (`E8126`).
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new("E8126", tonic::Code::Unimplemented, message)
    }

    /// Creates an internal error (`E8130`).
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("E8130", tonic::Code::Internal, message)
    }

    /// Creates a sandbox error (`E8132`).
    pub fn sandbox(message: impl Into<String>) -> Self {
        Self::new("E8132", tonic::Code::Internal, message)
    }

    /// Creates a deadline error (`E8131`).
    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new("E8131", tonic::Code::DeadlineExceeded, message)
    }

    fn new(code: &'static str, status: tonic::Code, message: impl Into<String>) -> Self {
        Self {
            code,
            status,
            message: message.into(),
        }
    }

    /// Returns the stable code.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Returns the gRPC status.
    pub fn status(&self) -> tonic::Code {
        self.status
    }
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ServerError {}

impl From<ServerError> for Status {
    fn from(error: ServerError) -> Self {
        Status::new(error.status, format!("{}: {}", error.code, error.message))
    }
}

impl From<seal_cas::CasError> for ServerError {
    fn from(error: seal_cas::CasError) -> Self {
        match error {
            seal_cas::CasError::InvalidDigest { .. } => Self::integrity(error.to_string()),
            _ => Self::internal(error.to_string()),
        }
    }
}

/// The result type for server operations.
pub type ServerResult<T> = Result<T, ServerError>;
