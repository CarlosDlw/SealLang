//! Running a harness process with captured output and an optional timeout.

use std::io::Read;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::CommandExt;

/// Everything one harness invocation produced.
pub(crate) struct Captured {
    /// The termination status.
    pub status: ExitStatus,
    /// Standard output, truncated only by the caller.
    pub stdout: Vec<u8>,
    /// Standard error, truncated only by the caller.
    pub stderr: Vec<u8>,
    /// Whether the runner killed the process at the deadline.
    pub timed_out: bool,
}

/// Runs `command`, capturing both streams.
///
/// Both readers run on their own threads so a test that writes more than a pipe
/// buffer cannot deadlock the parent while it waits for termination. On timeout
/// the child is killed and reaped before the readers are joined.
pub(crate) fn run(mut command: Command, timeout: Option<Duration>) -> std::io::Result<Captured> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    // A test may start helper processes. Putting the harness in its own process
    // group lets the timeout kill the whole tree instead of leaving a grandchild
    // holding the output pipes open.
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_reader = stdout.map(|mut stream| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stream.read_to_end(&mut bytes);
            bytes
        })
    });
    let err_reader = stderr.map(|mut stream| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stream.read_to_end(&mut bytes);
            bytes
        })
    });

    let deadline = timeout.map(|limit| Instant::now() + limit);
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            timed_out = true;
            kill(&mut child);
            break child.wait()?;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let stdout = out_reader
        .map(|reader| reader.join().unwrap_or_default())
        .unwrap_or_default();
    let stderr = err_reader
        .map(|reader| reader.join().unwrap_or_default())
        .unwrap_or_default();
    Ok(Captured {
        status,
        stdout,
        stderr,
        timed_out,
    })
}

/// Terminates the child and, on Unix, every process in its group.
#[cfg(unix)]
fn kill(child: &mut std::process::Child) {
    use rustix::process::{Pid, Signal, kill_process_group};

    if let Some(pid) = Pid::from_raw(child.id() as i32) {
        let _ = kill_process_group(pid, Signal::KILL);
    }
    let _ = child.kill();
}

/// Terminates the child.
#[cfg(not(unix))]
fn kill(child: &mut std::process::Child) {
    let _ = child.kill();
}

/// Returns the terminating signal on Unix, or `None` elsewhere.
#[cfg(unix)]
pub(crate) fn signal(status: ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

/// Returns the terminating signal on Unix, or `None` elsewhere.
#[cfg(not(unix))]
pub(crate) fn signal(_status: ExitStatus) -> Option<i32> {
    None
}
