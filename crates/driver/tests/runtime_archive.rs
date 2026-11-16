//! Command-line tests for exporting the embedded runtime archive.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static NEXT_OUTPUT: AtomicU32 = AtomicU32::new(0);

struct TempOutput(PathBuf);

impl TempOutput {
    fn new(suffix: &str) -> Self {
        Self(std::env::temp_dir().join(format!(
            "seal-driver-runtime-{}-{}-{suffix}",
            std::process::id(),
            NEXT_OUTPUT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}

impl Drop for TempOutput {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn emit_runtime_writes_the_embedded_archive_without_an_input() {
    let output = TempOutput::new("runtime.a");
    let result = Command::new(env!("CARGO_BIN_EXE_sealc"))
        .arg("--emit-runtime")
        .arg(&output.0)
        .output()
        .expect("the compiler runs");
    assert!(
        result.status.success(),
        "compiler failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let bytes = std::fs::read(&output.0).expect("the archive is readable");
    assert_eq!(bytes, seal_codegen::runtime_archive());
    assert!(!bytes.is_empty());
}

#[test]
fn exported_runtime_can_be_used_for_linking() {
    let archive = TempOutput::new("runtime.a");
    let executable = TempOutput::new("program");
    let export = Command::new(env!("CARGO_BIN_EXE_sealc"))
        .arg("--emit-runtime")
        .arg(&archive.0)
        .output()
        .expect("the compiler runs");
    assert!(
        export.status.success(),
        "runtime export failed: {}",
        String::from_utf8_lossy(&export.stderr)
    );

    let example =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/007_runtime_panic.sl");
    let compile = Command::new(env!("CARGO_BIN_EXE_sealc"))
        .arg("--runtime-archive")
        .arg(&archive.0)
        .arg(&example)
        .arg("-o")
        .arg(&executable.0)
        .output()
        .expect("the compiler runs");
    assert!(
        compile.status.success(),
        "linking with the exported runtime failed: {}",
        String::from_utf8_lossy(&compile.stderr)
    );

    let result = Command::new(&executable.0)
        .output()
        .expect("the executable runs");
    assert_eq!(result.status.code(), Some(101));
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("panic: attempt to divide by zero at "));
    assert!(stderr.contains("examples/007_runtime_panic.sl:5:3\n"));
}
