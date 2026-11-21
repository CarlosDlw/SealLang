//! The portable backend: empty environment, fixed working directory, timeout.
//!
//! It does not create a mount namespace, so it cannot hide host files or remove
//! network access. It is used only when the policy allows it and namespace
//! isolation is unavailable; callers report the weaker isolation to the user.

use std::ffi::OsStr;
use std::ffi::OsString;
use std::process::Command;

use crate::error::SandboxError;
use crate::policy::SandboxPolicy;
use crate::process;

/// Runs `program` with `arguments` without filesystem isolation.
pub(crate) fn run(
    program: &OsStr,
    arguments: &[OsString],
    policy: &SandboxPolicy,
) -> Result<process::Captured, SandboxError> {
    let mut command = Command::new(program);
    command.args(arguments);
    process::prepare(&mut command);
    for (key, value) in policy.environment() {
        command.env(key, value);
    }
    if let Some(directory) = policy.working_directory() {
        command.current_dir(directory);
    }
    process::capture(program, command, policy.wall_clock_timeout())
}
