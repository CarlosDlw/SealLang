//! Running declarative project tasks.
//!
//! Steps name a program and an argument vector; no shell interprets them. Each
//! step runs in the sandbox with the project root visible read-write, a cleared
//! environment, an explicit working directory, and an optional timeout. A
//! failing step stops the task unless the caller requests `--no-fail-fast`.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use seal_manifest::TaskManifest;
use seal_sandbox::{Sandbox, SandboxPolicy, SandboxRequest};

use crate::error::{BuildError, BuildResult};

/// One task selected by the CLI.
#[derive(Clone, Debug)]
pub struct TaskRequest {
    /// The task name.
    pub name: String,
    /// Extra arguments appended to every step.
    pub args: Vec<String>,
}

/// The result of one step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskOutcome {
    /// The zero-based step index.
    pub index: usize,
    /// The program that ran.
    pub program: String,
    /// The process exit code, when the process exited normally.
    pub status: Option<i32>,
    /// Whether the step succeeded.
    pub success: bool,
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
    /// Elapsed milliseconds.
    pub millis: u64,
}

/// The result of one task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskReport {
    /// The task name.
    pub name: String,
    /// One outcome per step that ran.
    pub outcomes: Vec<TaskOutcome>,
    /// The number of failed steps.
    pub failed: usize,
}

impl TaskReport {
    /// Returns whether every step succeeded.
    pub fn success(&self) -> bool {
        self.failed == 0
    }
}

/// Runs declarative tasks against one project root.
pub struct TaskRunner {
    root: std::path::PathBuf,
    sandbox: Sandbox,
    environment: BTreeMap<String, String>,
}

impl TaskRunner {
    /// Creates a runner for `root`.
    pub fn new(root: &Path, sandbox: Sandbox) -> Self {
        Self {
            root: root.to_path_buf(),
            sandbox,
            environment: BTreeMap::new(),
        }
    }

    /// Adds an environment value visible to every step.
    pub fn environment(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.environment.insert(key.into(), value.into());
        self
    }

    /// Runs `task`, stopping at the first failure unless `fail_fast` is false.
    pub fn run(
        &self,
        request: &TaskRequest,
        task: &TaskManifest,
        fail_fast: bool,
    ) -> BuildResult<TaskReport> {
        if task.steps.is_empty() {
            return Err(BuildError::task(format!(
                "the task `{}` has no steps",
                request.name
            )));
        }
        let mut outcomes = Vec::with_capacity(task.steps.len());
        let mut failed = 0;
        for (index, step) in task.steps.iter().enumerate() {
            let working_dir = match &step.cwd {
                Some(directory) => {
                    let path = self.root.join(directory);
                    let canonical = path.canonicalize().map_err(|error| {
                        BuildError::task(format!(
                            "the step {index} working directory `{}` is invalid: {error}",
                            path.display()
                        ))
                    })?;
                    if !canonical.starts_with(&self.root) {
                        return Err(BuildError::task(format!(
                            "the step {index} working directory `{}` leaves the project",
                            path.display()
                        )));
                    }
                    canonical
                }
                None => self.root.clone(),
            };
            let mut policy = SandboxPolicy::new()
                .read(&self.root)
                .write(&self.root)
                .working_dir(&working_dir)
                .allow_portable(true)
                .env("PATH", "/usr/bin:/bin");
            for (key, value) in &self.environment {
                policy = policy.env(key, value);
            }
            for (key, value) in &step.env {
                policy = policy.env(key, value);
            }
            if let Some(seconds) = step.timeout {
                policy = policy.timeout(Duration::from_secs(seconds));
            }
            let request_step = SandboxRequest::new(&step.program).policy(policy).args(
                step.args
                    .iter()
                    .cloned()
                    .chain(request.args.iter().cloned()),
            );
            let start = Instant::now();
            let output = self.sandbox.run(&request_step).map_err(|error| {
                BuildError::new(
                    error.code(),
                    format!(
                        "the task `{}` step {index} (`{}`) failed: {error}",
                        request.name, step.program
                    ),
                )
            })?;
            let success = output.success();
            outcomes.push(TaskOutcome {
                index,
                program: step.program.clone(),
                status: output.status.code(),
                success,
                stdout: output.stdout_text(),
                stderr: output.stderr_text(),
                millis: start.elapsed().as_millis().try_into().unwrap_or(u64::MAX),
            });
            if !success {
                failed += 1;
                if fail_fast {
                    break;
                }
            }
        }
        Ok(TaskReport {
            name: request.name.clone(),
            outcomes,
            failed,
        })
    }
}
