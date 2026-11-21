//! Tests, outcomes, and the report the runner produces.

use std::path::PathBuf;
use std::time::Duration;

/// Whether an executable contains tests or benchmarks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A test harness executable.
    Test,
    /// A benchmark harness executable.
    Bench,
}

impl Kind {
    /// Returns the stable spelling used in reports.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Test => "test",
            Self::Bench => "bench",
        }
    }
}

/// One test executable produced from a project target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Executable {
    /// The package that owns the target.
    pub package: String,
    /// The target name.
    pub target: String,
    /// The compiled executable.
    pub path: PathBuf,
    /// Whether it runs tests or benchmarks.
    pub kind: Kind,
}

/// One discovered test or benchmark.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Test {
    /// The package that owns the target.
    pub package: String,
    /// The target name.
    pub target: String,
    /// The executable that implements the protocol.
    pub executable: PathBuf,
    /// The fully qualified name printed by `--list`.
    pub name: String,
    /// Whether this is a test or a benchmark.
    pub kind: Kind,
}

/// The result of one attempt schedule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The test exited zero.
    Passed,
    /// The test exited non-zero or was killed by a signal.
    Failed,
    /// The test exceeded its timeout and was killed.
    Timeout,
    /// The test was not run because a filter or a prior failure stopped it.
    Skipped,
}

impl Status {
    /// Returns the stable spelling used in JSON and JUnit.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Timeout => "timeout",
            Self::Skipped => "skipped",
        }
    }
}

/// The final result of one logical test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The test name.
    pub name: String,
    /// The package that owns the test.
    pub package: String,
    /// The target that produced the executable.
    pub target: String,
    /// Whether this was a test or a benchmark.
    pub kind: Kind,
    /// The final status.
    pub status: Status,
    /// Wall time of the successful attempt, or of the last failed one.
    pub duration: Duration,
    /// The number of attempts, including the successful one.
    pub attempts: u32,
    /// A failure message, including captured output.
    pub message: String,
    /// The process exit code, when one exists.
    pub exit_code: Option<i32>,
    /// The terminating signal, on platforms that report one.
    pub signal: Option<i32>,
}

impl Outcome {
    /// Returns whether the test did not pass.
    pub const fn is_failure(&self) -> bool {
        matches!(self.status, Status::Failed | Status::Timeout)
    }
}

/// Everything one run produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// Outcomes in selection order, including skipped tests.
    pub outcomes: Vec<Outcome>,
    /// Total wall time of the run.
    pub duration: Duration,
}

impl Report {
    /// Returns the number of passed tests.
    pub fn passed(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|outcome| outcome.status == Status::Passed)
            .count()
    }

    /// Returns the number of failed and timed-out tests.
    pub fn failed(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|outcome| outcome.is_failure())
            .count()
    }

    /// Returns the number of skipped tests.
    pub fn skipped(&self) -> usize {
        self.outcomes
            .iter()
            .filter(|outcome| outcome.status == Status::Skipped)
            .count()
    }

    /// Returns whether any test failed.
    pub fn has_failures(&self) -> bool {
        self.failed() > 0
    }
}
