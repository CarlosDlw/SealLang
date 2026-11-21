//! Command-line surface tests for the subcommand interface.
//!
//! These run the real binary so the help layout, exit statuses, and messages
//! are exactly what a user sees.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT: AtomicU32 = AtomicU32::new(0);

fn sealc(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sealc"))
        .args(arguments)
        .output()
        .expect("the compiler runs")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(suffix: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "seal-driver-cli-{}-{}-{suffix}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("the temporary directory is created");
        Self(path)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn top_level_help_lists_every_command() {
    let output = sealc(&["--help"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let help = stdout(&output);
    for command in [
        "build",
        "run",
        "check",
        "explain",
        "emit-runtime",
        "version",
        "help",
    ] {
        assert!(
            help.contains(command),
            "top-level help is missing `{command}`: {help}"
        );
    }
}

#[test]
fn help_explains_one_command() {
    let output = sealc(&["help", "build"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let help = stdout(&output);
    assert!(
        help.contains("Usage: sealc build"),
        "unexpected build help: {help}"
    );
    assert!(
        help.contains("--emit"),
        "build help is missing --emit: {help}"
    );
}

#[test]
fn version_reports_the_compiler_language_and_host() {
    let output = sealc(&["version"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let version = stdout(&output);
    assert!(
        version.contains(env!("CARGO_PKG_VERSION")),
        "version output is missing the package version: {version}"
    );
    assert!(
        version.contains("language: 0.1"),
        "version output is missing the language version: {version}"
    );
    assert!(
        version.contains("host: "),
        "version output is missing the host triple: {version}"
    );
}

#[test]
fn run_builds_a_temporary_executable_and_returns_its_status() {
    let dir = TempDir::new("run-status");
    let source = dir.join("main.sl");
    fs::write(&source, "func main() -> i32 { 42 }\n").expect("the source is written");

    let output = sealc(&["run", source.to_str().expect("the path is UTF-8")]);
    assert_eq!(
        output.status.code(),
        Some(42),
        "run did not forward the program status: {}",
        stderr(&output)
    );
    assert!(stderr(&output).is_empty(), "unexpected diagnostics");

    let entries: Vec<String> = fs::read_dir(&dir.0)
        .expect("the directory is readable")
        .map(|entry| {
            entry
                .expect("the entry is readable")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(entries, vec!["main.sl".to_owned()], "run wrote an artifact");
}

#[cfg(unix)]
#[test]
fn run_removes_its_temporary_directory_on_success() {
    let root = TempDir::new("run-cleanup");
    let source = root.join("main.sl");
    fs::write(&source, "func main() -> i32 { 0 }\n").expect("the source is written");

    let output = Command::new(env!("CARGO_BIN_EXE_sealc"))
        .env("TMPDIR", &root.0)
        .arg("run")
        .arg(&source)
        .output()
        .expect("the compiler runs");
    assert!(output.status.success(), "{}", stderr(&output));

    let entries: Vec<String> = fs::read_dir(&root.0)
        .expect("the directory is readable")
        .map(|entry| {
            entry
                .expect("the entry is readable")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(
        entries,
        vec!["main.sl".to_owned()],
        "run left its temporary directory behind"
    );
}

#[test]
fn run_returns_a_compile_error_without_running_anything() {
    let dir = TempDir::new("run-error");
    let source = dir.join("main.sl");
    fs::write(&source, "func main() -> i32 {\n").expect("the source is written");

    let output = sealc(&["run", source.to_str().expect("the path is UTF-8")]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("error["),
        "run did not report the compile error: {}",
        stderr(&output)
    );
    assert_eq!(
        fs::read_dir(&dir.0)
            .expect("the directory is readable")
            .count(),
        1,
        "run left an artifact after a failed build"
    );
}

#[test]
fn run_returns_a_runtime_panic_status() {
    let dir = TempDir::new("run-panic");
    let source = dir.join("main.sl");
    fs::write(
        &source,
        "func main() -> i32 {\n    let divisor = 0;\n    10 / divisor\n}\n",
    )
    .expect("the source is written");

    let output = sealc(&["run", source.to_str().expect("the path is UTF-8")]);
    assert_eq!(output.status.code(), Some(101));
    assert!(
        stderr(&output).contains("panic: attempt to divide by zero"),
        "run did not forward the panic: {}",
        stderr(&output)
    );
}

#[test]
fn run_rejects_non_host_targets_before_building() {
    let output = sealc(&["run", "--target=wasm32-unknown-unknown", "main.sl"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("can only execute host programs"),
        "unexpected target error: {}",
        stderr(&output)
    );
}

#[test]
fn check_writes_no_artifact() {
    let dir = TempDir::new("check");
    let source = dir.join("main.sl");
    fs::write(&source, "func main() -> i32 { 42 }\n").expect("the source is written");

    let output = sealc(&["check", source.to_str().expect("the path is UTF-8")]);
    assert!(output.status.success(), "check failed: {}", stderr(&output));

    let entries: Vec<String> = fs::read_dir(&dir.0)
        .expect("the directory is readable")
        .map(|entry| {
            entry
                .expect("the entry is readable")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(
        entries,
        vec!["main.sl".to_owned()],
        "check wrote an artifact"
    );
}

#[test]
fn a_cross_object_has_the_requested_architecture() {
    let dir = TempDir::new("cross-object");
    let source = dir.join("main.sl");
    let object = dir.join("cross.o");
    fs::write(&source, "func main() -> i32 { 0 }\n").expect("source");
    let output = sealc(&[
        "build",
        "--target=aarch64-unknown-linux-gnu",
        "--emit=obj",
        "-o",
        object.to_str().expect("UTF-8"),
        source.to_str().expect("UTF-8"),
    ]);
    assert!(
        output.status.success(),
        "cross object failed: {}",
        stderr(&output)
    );
    let bytes = fs::read(&object).expect("object bytes");
    assert_eq!(&bytes[..4], b"\x7fELF");
    assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 183);
}

#[test]
fn usage_errors_exit_with_status_two() {
    for arguments in [
        &["build"][..],
        &["run"][..],
        &["check"][..],
        &["explain"][..],
        &["emit-runtime"][..],
        &["frobnicate"][..],
    ] {
        let output = sealc(arguments);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{arguments:?} should be a usage error: {}",
            stderr(&output)
        );
    }
}
