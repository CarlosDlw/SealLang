//! Small output helpers shared by the commands.

use std::process::ExitCode;

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
