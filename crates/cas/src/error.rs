//! CAS failures.

use std::fmt;

/// A failure in the content-addressed store or its action cache.
#[derive(Debug)]
pub enum CasError {
    /// A filesystem operation failed.
    Io {
        /// The path involved.
        path: String,
        /// The operation and reason.
        message: String,
    },
    /// A digest was not 64 hexadecimal characters.
    InvalidDigest {
        /// The rejected text.
        value: String,
        /// Why it is invalid.
        reason: String,
    },
    /// An object or action record has unexpected content.
    Corrupt {
        /// The object or record key.
        key: String,
        /// Why it is corrupt.
        reason: String,
    },
    /// A lock could not be acquired before the timeout.
    LockTimeout {
        /// The lock name.
        key: String,
    },
    /// A JSON action record could not be encoded.
    Serialization(String),
}

impl CasError {
    /// Creates an I/O failure.
    pub fn io(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Io {
            path: path.into(),
            message: message.into(),
        }
    }

    /// Creates an invalid digest failure.
    pub fn invalid_digest(value: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidDigest {
            value: value.into(),
            reason: reason.into(),
        }
    }

    /// Creates a corrupt-object failure.
    pub fn corrupt(key: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Corrupt {
            key: key.into(),
            reason: reason.into(),
        }
    }

    /// Returns the stable error code.
    pub fn code(&self) -> &'static str {
        "E8040"
    }
}

impl fmt::Display for CasError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => write!(f, "`{path}`: {message}"),
            Self::InvalidDigest { value, reason } => {
                write!(f, "invalid digest `{value}`: {reason}")
            }
            Self::Corrupt { key, reason } => write!(f, "cache entry `{key}` is corrupt: {reason}"),
            Self::LockTimeout { key } => {
                write!(f, "timed out waiting for the cache lock `{key}`")
            }
            Self::Serialization(message) => write!(f, "cannot encode cache metadata: {message}"),
        }
    }
}

impl std::error::Error for CasError {}
