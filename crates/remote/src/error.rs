//! Remote failures with stable diagnostic codes.
//!
//! Every error distinguishes configuration, transport, authentication,
//! integrity, and action failures. The build only falls back to local execution
//! for infrastructure failures; a remote action that ran and failed is a build
//! failure, never a reason to run the compiler a second time.

use std::fmt;

/// The category of a remote failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteErrorKind {
    /// Invalid endpoint, TLS, limits, or platform configuration.
    Configuration,
    /// The endpoint could not be reached or returned a transient status.
    Transport,
    /// The server rejected the bearer token.
    Authentication,
    /// A digest or size did not match the received bytes.
    Integrity,
    /// A requested blob or output is missing.
    MissingOutput,
    /// The command ran and exited unsuccessfully.
    ActionFailed,
    /// The server rejected an action-cache update.
    CacheUpdate,
    /// The server does not provide a required capability.
    Unsupported,
    /// The request exceeded its deadline.
    Timeout,
    /// The response violated the protocol.
    Protocol,
}

/// A remote or remote-execution failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteError {
    code: &'static str,
    kind: RemoteErrorKind,
    message: String,
    exit_code: Option<i32>,
}

impl RemoteError {
    /// Creates a configuration error (`E8120`).
    pub fn configuration(message: impl Into<String>) -> Self {
        Self::new("E8120", RemoteErrorKind::Configuration, message)
    }

    /// Creates a transport error (`E8121`).
    pub fn transport(message: impl Into<String>) -> Self {
        Self::new("E8121", RemoteErrorKind::Transport, message)
    }

    /// Creates an integrity error (`E8122`).
    pub fn integrity(message: impl Into<String>) -> Self {
        Self::new("E8122", RemoteErrorKind::Integrity, message)
    }

    /// Creates an authentication error (`E8123`).
    pub fn authentication(message: impl Into<String>) -> Self {
        Self::new("E8123", RemoteErrorKind::Authentication, message)
    }

    /// Creates a missing-output error (`E8124`).
    pub fn missing_output(message: impl Into<String>) -> Self {
        Self::new("E8124", RemoteErrorKind::MissingOutput, message)
    }

    /// Creates an action-failure error (`E8125`).
    pub fn action_failed(exit_code: i32, message: impl Into<String>) -> Self {
        let mut error = Self::new("E8125", RemoteErrorKind::ActionFailed, message);
        error.exit_code = Some(exit_code);
        error
    }

    /// Creates an unsupported-capability error (`E8126`).
    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new("E8126", RemoteErrorKind::Unsupported, message)
    }

    /// Creates a cache-update error (`E8127`).
    pub fn cache_update(message: impl Into<String>) -> Self {
        Self::new("E8127", RemoteErrorKind::CacheUpdate, message)
    }

    /// Creates a remote-required failure (`E8129`).
    pub fn required(message: impl Into<String>) -> Self {
        Self::new("E8129", RemoteErrorKind::Unsupported, message)
    }

    /// Creates a timeout error (`E8131`).
    pub fn timeout(message: impl Into<String>) -> Self {
        Self::new("E8131", RemoteErrorKind::Timeout, message)
    }

    /// Creates a protocol error (`E8130`).
    pub fn protocol(message: impl Into<String>) -> Self {
        Self::new("E8130", RemoteErrorKind::Protocol, message)
    }

    fn new(code: &'static str, kind: RemoteErrorKind, message: impl Into<String>) -> Self {
        Self {
            code,
            kind,
            message: message.into(),
            exit_code: None,
        }
    }

    /// Returns the stable diagnostic code.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// Returns the failure category.
    pub fn kind(&self) -> RemoteErrorKind {
        self.kind
    }

    /// Returns the message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns whether this failure means the command ran and failed.
    pub fn is_action_failure(&self) -> bool {
        self.kind == RemoteErrorKind::ActionFailed
    }

    /// Returns the remote exit code for an action failure.
    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }
}

impl fmt::Display for RemoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for RemoteError {}

impl From<tonic::Status> for RemoteError {
    fn from(status: tonic::Status) -> Self {
        let message = status.message().to_owned();
        match status.code() {
            tonic::Code::Unauthenticated | tonic::Code::PermissionDenied => {
                Self::authentication(message)
            }
            tonic::Code::DeadlineExceeded => Self::timeout(message),
            tonic::Code::Unavailable | tonic::Code::ResourceExhausted => Self::transport(message),
            tonic::Code::NotFound => Self::missing_output(message),
            tonic::Code::Unimplemented => Self::unsupported(message),
            tonic::Code::InvalidArgument => Self::configuration(message),
            _ => Self::protocol(message),
        }
    }
}

impl From<tonic::transport::Error> for RemoteError {
    fn from(error: tonic::transport::Error) -> Self {
        Self::transport(error.to_string())
    }
}

impl From<seal_cas::CasError> for RemoteError {
    fn from(error: seal_cas::CasError) -> Self {
        match error {
            seal_cas::CasError::InvalidDigest { .. } => Self::integrity(error.to_string()),
            _ => Self::configuration(error.to_string()),
        }
    }
}

/// The result type for remote operations.
pub type RemoteResult<T> = Result<T, RemoteError>;
