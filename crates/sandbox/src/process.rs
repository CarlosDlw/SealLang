//! Starting a child, capturing its streams, and enforcing a timeout.

use std::ffi::OsStr;
use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::error::SandboxError;

/// The captured result of a sandboxed command.
pub(crate) struct Captured {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
}

/// Configures a command to use piped streams and an empty environment.
pub(crate) fn prepare(command: &mut Command) {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear();
}

/// Spawns `command`, captures both streams, and enforces `timeout`.
pub(crate) fn capture(
    program: &OsStr,
    mut command: Command,
    timeout: Option<Duration>,
) -> Result<Captured, SandboxError> {
    let display = program.to_string_lossy().into_owned();
    let mut child = command
        .spawn()
        .map_err(|error| SandboxError::io(&display, format!("cannot start: {error}")))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    std::thread::scope(|scope| {
        let out_reader = stdout.map(|stream| scope.spawn(move || read_stream(stream)));
        let err_reader = stderr.map(|stream| scope.spawn(move || read_stream(stream)));
        let status = wait(&mut child, &display, timeout)?;
        let stdout = join(out_reader)?;
        let stderr = join(err_reader)?;
        Ok::<Captured, SandboxError>(Captured {
            status,
            stdout,
            stderr,
        })
    })
}

fn wait(
    child: &mut Child,
    program: &str,
    timeout: Option<Duration>,
) -> Result<ExitStatus, SandboxError> {
    let Some(limit) = timeout else {
        return child
            .wait()
            .map_err(|error| SandboxError::io(program, format!("cannot wait: {error}")));
    };
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(error) => {
                return Err(SandboxError::io(
                    program,
                    format!("cannot observe the process: {error}"),
                ));
            }
        }
        if Instant::now() >= deadline {
            // `kill` may fail if the process raced to completion; the following
            // `wait` still reaps it and reports the real status.
            let _ = child.kill();
            let _ = child.wait();
            return Err(SandboxError::Timeout {
                program: program.to_owned(),
                milliseconds: limit.as_millis().try_into().unwrap_or(u64::MAX),
            });
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn read_stream(mut stream: impl Read) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn join<T>(
    handle: Option<std::thread::ScopedJoinHandle<'_, std::io::Result<T>>>,
) -> Result<T, SandboxError>
where
    T: Default,
{
    match handle {
        None => Ok(T::default()),
        Some(handle) => match handle.join() {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(SandboxError::io(
                "<pipe>",
                format!("cannot read process output: {error}"),
            )),
            Err(_) => Err(SandboxError::io("<pipe>", "a stream reader panicked")),
        },
    }
}

/// Turns an unsuccessful exit status into a typed failure with captured stderr.
pub(crate) fn check_status(
    program: &OsStr,
    status: ExitStatus,
    stderr: &[u8],
) -> Result<(), SandboxError> {
    if status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(stderr);
    Err(SandboxError::Failed {
        program: program.to_string_lossy().into_owned(),
        status: status.to_string(),
        stderr: stderr.trim().chars().take(4_000).collect(),
    })
}
