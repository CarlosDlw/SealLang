//! Sandbox failures.

use std::fmt;

/// A failure while preparing or running a sandboxed command.
#[derive(Debug)]
pub enum SandboxError {
    /// The requested isolation cannot be provided on this host.
    Unsupported {
        /// Why isolation is unavailable.
        reason: String,
    },
    /// A policy path could not be resolved or created.
    Policy {
        /// The path involved.
        path: String,
        /// Why the policy is invalid.
        reason: String,
    },
    /// A process could not be started or observed.
    Io {
        /// The program or path involved.
        program: String,
        /// The operation and reason.
        message: String,
    },
    /// The process exceeded its timeout and was killed.
    Timeout {
        /// The program that timed out.
        program: String,
        /// The configured limit in milliseconds.
        milliseconds: u64,
    },
    /// The process ran and exited unsuccessfully.
    Failed {
        /// The program that failed.
        program: String,
        /// The human-readable exit status.
        status: String,
        /// Captured standard error, trimmed.
        stderr: String,
    },
}

impl SandboxError {
    /// Returns the stable error code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Timeout { .. } => "E8044",
            Self::Unsupported { .. }
            | Self::Policy { .. }
            | Self::Io { .. }
            | Self::Failed { .. } => "E8041",
        }
    }

    pub(crate) fn io(program: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Io {
            program: program.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { reason } => write!(f, "sandbox unavailable: {reason}"),
            Self::Policy { path, reason } => write!(f, "invalid sandbox path `{path}`: {reason}"),
            Self::Io { program, message } => write!(f, "`{program}`: {message}"),
            Self::Timeout {
                program,
                milliseconds,
            } => write!(
                f,
                "`{program}` exceeded the sandbox timeout of {milliseconds} ms and was killed"
            ),
            Self::Failed {
                program,
                status,
                stderr,
            } => {
                if stderr.is_empty() {
                    write!(f, "`{program}` failed ({status})")
                } else {
                    write!(f, "`{program}` failed ({status}): {stderr}")
                }
            }
        }
    }
}

impl std::error::Error for SandboxError {}
