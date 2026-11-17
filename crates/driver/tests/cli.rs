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
fn run_reports_that_it_is_not_implemented() {
    let output = sealc(&["run", "main.sl"]);
    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(
        message.contains("not implemented yet"),
        "unexpected run message: {message}"
    );
    assert!(
        message.contains("sealc build"),
        "the run message should point at build: {message}"
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
fn usage_errors_exit_with_status_two() {
    for arguments in [
        &["build"][..],
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
