//! Runs the heap ABI contract tests against the embedded runtime archive.
//!
//! The harness is C because the ABI is a C boundary. It is compiled with the
//! host C toolchain, linked against the exact archive bytes embedded in the
//! compiler, and executed as a real process.
//!
//! The test process does not inherit Cargo's build-script environment
//! (`TARGET`/`OPT_LEVEL`), so `cc::Build` cannot discover a compiler here.
//! Invoking `CC` (or the platform default) directly keeps the test independent
//! of build-script metadata.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

/// A temporary directory removed on every exit path, including panics.
struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the system clock is after the Unix epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!("seal-heap-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path).expect("the temporary directory is created");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Runs a command and reports its captured output when it fails.
fn run(command: &mut Command) -> Output {
    let output = command.output().expect("the tool process starts");
    assert!(
        output.status.success(),
        "command failed: {command:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// Returns the configured C compiler or the platform default.
fn compiler() -> PathBuf {
    if let Some(compiler) = env::var_os("CC") {
        return PathBuf::from(compiler);
    }
    if cfg!(windows) {
        PathBuf::from("cl")
    } else {
        PathBuf::from("cc")
    }
}

/// Returns whether the compiler uses MSVC command-line conventions.
fn is_msvc(compiler: &Path) -> bool {
    let name = compiler
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    name == "cl" || name == "clang-cl"
}

#[test]
fn the_heap_abi_satisfies_its_contract() {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let temp = TempDirectory::new();
    let compiler = compiler();
    let msvc = is_msvc(&compiler);
    let archive = temp
        .path()
        .join(if msvc { "seal_rt.lib" } else { "seal_rt.a" });
    fs::write(&archive, seal_codegen::runtime_archive()).expect("the runtime archive is written");

    let source = Path::new(manifest).join("tests/runtime/heap.c");
    let include = Path::new(manifest).join("runtime/include");
    let object = temp.path().join(if msvc { "heap.obj" } else { "heap.o" });
    let mut compile = Command::new(&compiler);
    if msvc {
        compile
            .arg("/nologo")
            .arg("/c")
            .arg("/I")
            .arg(&include)
            .arg(format!("/Fo{}", object.display()))
            .arg(&source);
    } else {
        compile
            .arg("-std=c11")
            .arg("-O2")
            .arg("-I")
            .arg(&include)
            .arg("-c")
            .arg(&source)
            .arg("-o")
            .arg(&object);
    }
    run(&mut compile);

    let executable = temp.path().join(if cfg!(windows) {
        "heap_test.exe"
    } else {
        "heap_test"
    });
    let mut link = Command::new(&compiler);
    if msvc {
        link.arg("/nologo")
            .arg(format!("/Fe:{}", executable.display()))
            .arg(&object)
            .arg(&archive);
    } else {
        link.arg(&object).arg(&archive).arg("-o").arg(&executable);
    }
    run(&mut link);

    let mut execute = Command::new(&executable);
    run(&mut execute);
}
