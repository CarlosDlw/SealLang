//! Server failures mapped onto the registry's stable diagnostic codes.

use std::fmt;

use seal_registry::RegistryError;

/// A server failure with an HTTP status and a stable code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerError {
    status: u16,
    code: &'static str,
    message: String,
}

impl ServerError {
    /// Creates an error with an explicit status and code.
    pub fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    /// A malformed request (`400`, `E8093`).
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(400, "E8093", message)
    }

    /// The caller is not authenticated (`401`, `E8084`).
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(401, "E8084", message)
    }

    /// The caller is authenticated but not allowed (`403`, `E8083`).
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(403, "E8083", message)
    }

    /// The requested object does not exist (`404`, `E8025`).
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(404, "E8025", message)
    }

    /// The version already exists (`409`, `E8082`).
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(409, "E8082", message)
    }

    /// A checksum or signature failed (`422`, `E8021`).
    pub fn integrity(message: impl Into<String>) -> Self {
        Self::new(422, "E8021", message)
    }

    /// Provenance is missing or invalid (`422`, `E8096`).
    pub fn provenance(message: impl Into<String>) -> Self {
        Self::new(422, "E8096", message)
    }

    /// The transparency log failed (`422`, `E8097`).
    pub fn transparency(message: impl Into<String>) -> Self {
        Self::new(422, "E8097", message)
    }

    /// A quota or rate limit was exceeded (`429`, `E8098`).
    pub fn quota(message: impl Into<String>) -> Self {
        Self::new(429, "E8098", message)
    }

    /// The server could not complete the operation (`500`, `E8099`).
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(500, "E8099", message)
    }

    /// Returns the HTTP status.
    pub fn status(&self) -> u16 {
        self.status
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

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ServerError {}

impl From<RegistryError> for ServerError {
    fn from(error: RegistryError) -> Self {
        let status = match error.code() {
            "E8084" => 401,
            "E8083" => 403,
            "E8082" => 409,
            "E8085" => 503,
            "E8027" | "E8086" => 422,
            _ => 400,
        };
        let code = match error.code() {
            "E8021" => "E8021",
            "E8025" => "E8025",
            "E8027" => "E8093",
            "E8037" => "E8096",
            "E8080" => "E8080",
            "E8082" => "E8082",
            "E8083" => "E8083",
            "E8084" => "E8084",
            "E8093" => "E8093",
            "E8096" => "E8096",
            "E8097" => "E8097",
            "E8098" => "E8098",
            _ => "E8099",
        };
        Self::new(status, code, error.message())
    }
}

impl From<std::io::Error> for ServerError {
    fn from(error: std::io::Error) -> Self {
        Self::internal(format!("filesystem error: {error}"))
    }
}

/// The server result type.
pub type ServerResult<T> = Result<T, ServerError>;
