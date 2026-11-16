//! Command-line tests for executable linking.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT: AtomicU32 = AtomicU32::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(suffix: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "seal-driver-link-{}-{}-{suffix}",
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
fn a_failed_link_preserves_the_existing_output_and_leaves_no_temporary_files() {
    let dir = TempDir::new("failed-link");
    let source = dir.join("main.sl");
    fs::write(&source, "func main() -> i32 { 0 }\n").expect("the source is written");
    // An explicit extension keeps the driver from rewriting the name on
    // Windows, so the same assertions hold on every host.
    let output = dir.join("program.bin");
    fs::write(&output, b"previous build").expect("the previous output is written");

    let result = Command::new(env!("CARGO_BIN_EXE_sealc"))
        .env("SEAL_LINKER", "seal-missing-linker")
        .arg(&source)
        .arg("-o")
        .arg(&output)
        .output()
        .expect("the compiler runs");

    assert!(
        !result.status.success(),
        "a missing linker must fail the build"
    );
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("E6003") && stderr.contains("cannot run linker"),
        "unexpected stderr: {stderr}"
    );
    assert_eq!(
        fs::read(&output).expect("the previous output is readable"),
        b"previous build",
        "a failed link modified the previous output"
    );

    let mut entries: Vec<String> = fs::read_dir(&dir.0)
        .expect("the directory is readable")
        .map(|entry| {
            entry
                .expect("the entry is readable")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    entries.sort();
    let mut expected = vec!["main.sl".to_owned(), "program.bin".to_owned()];
    expected.sort();
    assert_eq!(entries, expected, "temporary files were left behind");
}
