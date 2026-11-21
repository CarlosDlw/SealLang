//! Isolated, parallel execution of scheduled tests.

use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use crate::error::TestError;
use crate::filter::Scheduled;
use crate::model::{Kind, Outcome, Report, Status};
use crate::process;

/// Maximum bytes of standard output kept in a failure message.
const OUTPUT_LIMIT: usize = 64 * 1024;

/// A callback invoked as soon as each outcome is known.
pub type Progress = dyn Fn(&Outcome) + Send + Sync;

/// Options for one scheduled run.
#[derive(Clone)]
pub struct Options {
    /// Maximum tests running at once.
    pub jobs: usize,
    /// Whether a failure stops scheduling new tests.
    pub fail_fast: bool,
    /// Number of retries after a failed or timed-out attempt.
    pub retries: u32,
    /// Per-attempt timeout.
    pub timeout: Option<Duration>,
    /// Benchmark iterations passed to the harness.
    pub iterations: u32,
    /// Working directory for every harness invocation.
    pub directory: Option<std::path::PathBuf>,
    /// Optional live progress callback.
    pub progress: Option<Arc<Progress>>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            jobs: std::thread::available_parallelism()
                .map(|value| value.get())
                .unwrap_or(1),
            fail_fast: true,
            retries: 0,
            timeout: Some(Duration::from_secs(60)),
            iterations: 100,
            directory: None,
            progress: None,
        }
    }
}

/// Runs every scheduled test and returns the ordered report.
pub fn run(scheduled: &[Scheduled], options: &Options) -> Result<Report, TestError> {
    if options.jobs == 0 {
        return Err(TestError::option("`--jobs` must be at least one"));
    }
    if options.timeout.is_some_and(|timeout| timeout.is_zero()) {
        return Err(TestError::option("`--timeout` must be at least one second"));
    }
    let started = Instant::now();
    let next = AtomicUsize::new(0);
    let stopped = AtomicBool::new(false);
    let mut outcomes: Vec<Option<Outcome>> = (0..scheduled.len()).map(|_| None).collect();
    let slots: Vec<std::sync::Mutex<Option<Outcome>>> = scheduled
        .iter()
        .map(|_| std::sync::Mutex::new(None))
        .collect();

    std::thread::scope(|scope| {
        for _ in 0..options.jobs.min(scheduled.len().max(1)) {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= scheduled.len() {
                        break;
                    }
                    if options.fail_fast && stopped.load(Ordering::Relaxed) {
                        let scheduled = &scheduled[index];
                        let outcome = skipped(scheduled, "not run after a failure");
                        if let Some(progress) = &options.progress {
                            progress(&outcome);
                        }
                        *slots[index].lock().expect("slot lock") = Some(outcome);
                        continue;
                    }
                    let outcome = attempt_with_retries(&scheduled[index], options);
                    if outcome.is_failure() && options.fail_fast {
                        stopped.store(true, Ordering::Relaxed);
                    }
                    if let Some(progress) = &options.progress {
                        progress(&outcome);
                    }
                    *slots[index].lock().expect("slot lock") = Some(outcome);
                }
            });
        }
    });

    for (index, slot) in slots.into_iter().enumerate() {
        outcomes[index] = slot.into_inner().expect("slot lock");
    }
    Ok(Report {
        outcomes: outcomes.into_iter().flatten().collect(),
        duration: started.elapsed(),
    })
}

/// Runs one scheduled test, retrying failed and timed-out attempts.
fn attempt_with_retries(scheduled: &Scheduled, options: &Options) -> Outcome {
    let mut attempts = 0;
    loop {
        attempts += 1;
        let outcome = attempt(scheduled, options, attempts);
        if !outcome.is_failure() || attempts > options.retries {
            return outcome;
        }
    }
}

/// Runs one attempt of one scheduled test.
fn attempt(scheduled: &Scheduled, options: &Options, attempts: u32) -> Outcome {
    let test = &scheduled.test;
    let mut command = Command::new(&test.executable);
    match test.kind {
        Kind::Test => {
            command.arg("--run").arg(&test.name);
        }
        Kind::Bench => {
            command
                .arg("--bench")
                .arg(&test.name)
                .arg(options.iterations.to_string());
        }
    }
    if let Some(directory) = &options.directory {
        command.current_dir(directory);
    }
    let started = Instant::now();
    let captured = process::run(command, options.timeout);
    let duration = started.elapsed();
    let mut outcome = Outcome {
        name: scheduled.label.clone(),
        package: test.package.clone(),
        target: test.target.clone(),
        kind: test.kind,
        status: Status::Passed,
        duration,
        attempts,
        message: String::new(),
        exit_code: None,
        signal: None,
    };
    match captured {
        Err(error) => {
            outcome.status = Status::Failed;
            outcome.message = format!("cannot execute the harness: {error}");
        }
        Ok(captured) if captured.timed_out => {
            outcome.status = Status::Timeout;
            outcome.exit_code = captured.status.code();
            outcome.signal = process::signal(captured.status);
            outcome.message = format!(
                "timed out after {}",
                options
                    .timeout
                    .map(|timeout| format!("{}s", timeout.as_secs()))
                    .unwrap_or_else(|| "the deadline".to_owned())
            );
        }
        Ok(captured) => {
            outcome.exit_code = captured.status.code();
            outcome.signal = process::signal(captured.status);
            if captured.status.success() {
                outcome.status = Status::Passed;
            } else {
                outcome.status = Status::Failed;
                outcome.message = failure_message(&captured.stdout, &captured.stderr);
                if outcome.message.is_empty() {
                    outcome.message = match (outcome.exit_code, outcome.signal) {
                        (Some(code), _) => format!("exited with status {code}"),
                        (_, Some(signal)) => format!("terminated by signal {signal}"),
                        _ => "terminated without an exit status".to_owned(),
                    };
                }
            }
        }
    }
    outcome
}

/// Builds a failure message from captured streams, bounded in size.
fn failure_message(stdout: &[u8], stderr: &[u8]) -> String {
    let mut message = String::new();
    if !stderr.is_empty() {
        message.push_str(&truncate(&String::from_utf8_lossy(stderr)));
    }
    if !stdout.is_empty() {
        if !message.is_empty() {
            message.push('\n');
        }
        message.push_str(&truncate(&String::from_utf8_lossy(stdout)));
    }
    message.trim().to_owned()
}

fn truncate(text: &str) -> String {
    if text.len() <= OUTPUT_LIMIT {
        return text.to_owned();
    }
    let mut end = OUTPUT_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… (output truncated)", &text[..end])
}

fn skipped(scheduled: &Scheduled, message: &str) -> Outcome {
    Outcome {
        name: scheduled.label.clone(),
        package: scheduled.test.package.clone(),
        target: scheduled.test.target.clone(),
        kind: scheduled.test.kind,
        status: Status::Skipped,
        duration: Duration::ZERO,
        attempts: 0,
        message: message.to_owned(),
        exit_code: None,
        signal: None,
    }
}
