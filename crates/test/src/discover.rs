//! Discovering the tests one executable contains.

use std::process::Command;
use std::time::Duration;

use crate::error::TestError;
use crate::model::{Executable, Test};
use crate::process;

/// Maximum time a harness may take to list its tests.
const LIST_TIMEOUT: Duration = Duration::from_secs(60);

/// Runs `--list` on every executable and returns the discovered tests.
pub fn discover(executables: &[Executable]) -> Result<Vec<Test>, TestError> {
    let mut tests = Vec::new();
    for executable in executables {
        let mut command = Command::new(&executable.path);
        command.arg("--list");
        let captured = process::run(command, Some(LIST_TIMEOUT)).map_err(|error| {
            TestError::harness(
                executable.path.display().to_string(),
                format!("cannot start the harness: {error}"),
            )
        })?;
        if captured.timed_out {
            return Err(TestError::harness(
                executable.path.display().to_string(),
                "the harness did not answer `--list` before the timeout",
            ));
        }
        if !captured.status.success() {
            let detail = String::from_utf8_lossy(&captured.stderr);
            return Err(TestError::harness(
                executable.path.display().to_string(),
                format!(
                    "`--list` exited with {}: {}",
                    captured.status,
                    detail.trim()
                ),
            ));
        }
        let text = String::from_utf8_lossy(&captured.stdout);
        for line in text.lines() {
            let name = line.trim();
            if name.is_empty() {
                continue;
            }
            if name.contains('\0') {
                return Err(TestError::harness(
                    executable.path.display().to_string(),
                    "`--list` printed a name containing a NUL byte",
                ));
            }
            tests.push(Test {
                package: executable.package.clone(),
                target: executable.target.clone(),
                executable: executable.path.clone(),
                name: name.to_owned(),
                kind: executable.kind,
            });
        }
    }
    Ok(tests)
}
