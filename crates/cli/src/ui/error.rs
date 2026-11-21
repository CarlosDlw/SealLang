//! Small output helpers shared by the commands.

use std::process::ExitCode;

/// A user-facing failure that has not been printed yet.
#[derive(Clone, Debug)]
pub(crate) struct Failure {
    code: String,
    message: String,
}

impl Failure {
    /// Prints the failure and returns the standard exit status.
    pub(crate) fn exit(self) -> ExitCode {
        error(&self.code, &self.message)
    }
}

/// Creates an unprinted failure.
pub(crate) fn failure(code: &str, message: impl Into<String>) -> Failure {
    Failure {
        code: code.to_owned(),
        message: message.into(),
    }
}

/// Reports a user-facing error and returns the failure status.
pub(crate) fn error(code: &str, message: &str) -> ExitCode {
    eprintln!("error[{code}]: {message}");
    ExitCode::from(1)
}

/// Reports a warning without failing the command.
pub(crate) fn warning(message: &str) {
    eprintln!("warning: {message}");
}

/// Returns the process exit status for a shell status.
pub(crate) fn exit_code(status: std::process::ExitStatus) -> ExitCode {
    status
        .code()
        .and_then(|code| u8::try_from(code).ok())
        .map_or_else(|| ExitCode::from(1), ExitCode::from)
}
