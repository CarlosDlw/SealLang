//! Shared helpers for command-line acceptance and diagnostic tests.
//!
//! Every helper runs the real compiler binary, so a test observes exactly what
//! a user sees: the exit status and the rendered diagnostics.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT_FILE: AtomicU32 = AtomicU32::new(0);

/// A temporary `.sl` source file removed on drop.
pub(crate) struct SourceFile(PathBuf);

impl SourceFile {
    pub(crate) fn new(name: &str, source: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "seal-diagnostics-{}-{}-{name}.sl",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, source).expect("the test source is writable");
        Self(path)
    }

    /// Type-checks the file and captures the compiler output.
    pub(crate) fn check(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_sealc"))
            .arg("check")
            .arg(&self.0)
            .output()
            .expect("the compiler runs")
    }
}

impl Drop for SourceFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Runs `source` and requires a compilation error with `code` and `fragment`.
pub(crate) fn reject(name: &str, source: &str, code: &str, fragment: &str) {
    let file = SourceFile::new(name, source);
    let output = file.check();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{name}: expected exit status 1, got {:?}: {stderr}",
        output.status
    );
    assert!(
        stderr.contains(&format!("error[{code}]")),
        "{name}: expected {code}, got: {stderr}"
    );
    assert!(
        stderr.contains(fragment),
        "{name}: expected {fragment:?} in: {stderr}"
    );
}
