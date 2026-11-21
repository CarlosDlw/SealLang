//! Runner behavior against real child processes.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use seal_test::{Kind, Options, Scheduled, Status, Test};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A temporary directory with an executable script.
struct Script {
    directory: PathBuf,
    path: PathBuf,
}

impl Drop for Script {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

impl Script {
    fn new(name: &str, body: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!(
            "seal-test-runner-{}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            name
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("temporary directory");
        let path = directory.join("script");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("script");
        let mut permissions = std::fs::metadata(&path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&path, permissions).expect("permissions");
        Self { directory, path }
    }

    fn scheduled(&self, name: &str) -> Scheduled {
        Scheduled {
            test: Test {
                package: "fixture".to_owned(),
                target: "script".to_owned(),
                executable: self.path.clone(),
                name: name.to_owned(),
                kind: Kind::Test,
            },
            label: name.to_owned(),
        }
    }
}

fn options(directory: &Path) -> Options {
    Options {
        jobs: 1,
        fail_fast: true,
        retries: 0,
        timeout: Some(Duration::from_secs(5)),
        iterations: 1,
        directory: Some(directory.to_path_buf()),
        progress: None,
    }
}

#[test]
fn a_zero_exit_is_a_pass() {
    let script = Script::new("pass", "exit 0");
    let report =
        seal_test::run(&[script.scheduled("pass")], &options(&script.directory)).expect("runner");
    assert_eq!(report.outcomes[0].status, Status::Passed);
    assert_eq!(report.passed(), 1);
}

#[test]
fn captured_stderr_explains_a_failure() {
    let script = Script::new("fail", "echo 'bad value' >&2\nexit 3");
    let report =
        seal_test::run(&[script.scheduled("fail")], &options(&script.directory)).expect("runner");
    assert_eq!(report.outcomes[0].status, Status::Failed);
    assert_eq!(report.outcomes[0].exit_code, Some(3));
    assert!(report.outcomes[0].message.contains("bad value"));
}

#[test]
fn a_retry_recovers_a_flaky_test() {
    let script = Script::new(
        "flaky",
        "if [ -f state ]; then exit 0; else touch state; exit 1; fi",
    );
    let mut settings = options(&script.directory);
    settings.retries = 1;
    let report = seal_test::run(&[script.scheduled("flaky")], &settings).expect("runner");
    assert_eq!(report.outcomes[0].status, Status::Passed);
    assert_eq!(report.outcomes[0].attempts, 2);
}

#[test]
fn a_signal_is_reported_as_a_failure() {
    let script = Script::new("signal", "kill -TERM $$");
    let report =
        seal_test::run(&[script.scheduled("signal")], &options(&script.directory)).expect("runner");
    assert_eq!(report.outcomes[0].status, Status::Failed);
    assert_eq!(report.outcomes[0].signal, Some(15));
    assert!(report.outcomes[0].message.contains("signal 15"));
}

#[test]
fn a_timeout_kills_the_child() {
    let script = Script::new("timeout", "sleep 30");
    let mut settings = options(&script.directory);
    settings.timeout = Some(Duration::from_millis(100));
    let report = seal_test::run(&[script.scheduled("timeout")], &settings).expect("runner");
    assert_eq!(report.outcomes[0].status, Status::Timeout);
}

#[test]
fn fail_fast_marks_the_remaining_tests_skipped() {
    let script = Script::new("fail", "exit 1");
    let scheduled = [
        script.scheduled("first"),
        script.scheduled("second"),
        script.scheduled("third"),
    ];
    let report = seal_test::run(&scheduled, &options(&script.directory)).expect("runner");
    assert_eq!(report.outcomes[0].status, Status::Failed);
    assert_eq!(report.outcomes[1].status, Status::Skipped);
    assert_eq!(report.outcomes[2].status, Status::Skipped);
}

#[test]
fn no_fail_fast_runs_every_test() {
    let script = Script::new("fail", "exit 1");
    let scheduled = [script.scheduled("first"), script.scheduled("second")];
    let mut settings = options(&script.directory);
    settings.fail_fast = false;
    let report = seal_test::run(&scheduled, &settings).expect("runner");
    assert_eq!(report.failed(), 2);
    assert_eq!(report.skipped(), 0);
}
