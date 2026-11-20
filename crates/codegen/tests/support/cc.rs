//! Shared C toolchain helpers for integration tests.
//!
//! The test process does not inherit Cargo's build-script environment
//! (`TARGET`/`OPT_LEVEL`), so `cc::Build` cannot discover a compiler here.
//! Invoking `CC` (or the platform default) directly keeps the tests independent
//! of build-script metadata. Every command is run as an argument vector, never
//! through a shell.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

/// A temporary directory removed on every exit path, including panics.
pub(crate) struct TempDirectory(PathBuf);

impl TempDirectory {
    /// Creates a unique directory whose name starts with `prefix`.
    pub(crate) fn new(prefix: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the system clock is after the Unix epoch")
            .as_nanos();
        let path = env::temp_dir().join(format!("seal-{prefix}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path).expect("the temporary directory is created");
        Self(path)
    }

    /// Returns the directory.
    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Runs a command and reports its captured output when it fails.
pub(crate) fn run(command: &mut Command) -> Output {
    let output = command.output().expect("the tool process starts");
    assert!(
        output.status.success(),
        "command failed: {command:?}\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// The C compiler used by a test and its command-line dialect.
pub(crate) struct Compiler {
    program: PathBuf,
    msvc: bool,
}

impl Compiler {
    /// Uses `CC`, or the platform default.
    pub(crate) fn discover() -> Self {
        let program = if let Some(compiler) = env::var_os("CC") {
            PathBuf::from(compiler)
        } else if cfg!(windows) {
            PathBuf::from("cl")
        } else {
            PathBuf::from("cc")
        };
        let name = program
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let msvc = name == "cl" || name == "clang-cl";
        Self { program, msvc }
    }

    /// Returns whether this compiler uses MSVC conventions.
    pub(crate) fn is_msvc(&self) -> bool {
        self.msvc
    }

    /// Compiles `source` to `object`, optionally adding `include`.
    pub(crate) fn compile_object(&self, source: &Path, include: Option<&Path>, object: &Path) {
        let mut command = Command::new(&self.program);
        if self.msvc {
            command.arg("/nologo").arg("/W4").arg("/c");
            if let Some(include) = include {
                command.arg("/I").arg(include);
            }
            command.arg(format!("/Fo{}", object.display())).arg(source);
        } else {
            command
                .arg("-std=c11")
                .arg("-O2")
                .arg("-Wall")
                .arg("-Wextra")
                .arg("-Wpedantic")
                .arg("-c");
            if let Some(include) = include {
                command.arg("-I").arg(include);
            }
            command.arg(source).arg("-o").arg(object);
        }
        run(&mut command);
    }

    /// Archives one object into `archive`.
    pub(crate) fn archive(&self, object: &Path, archive: &Path) {
        let mut command = if self.msvc {
            Command::new("lib")
        } else {
            Command::new(env::var_os("AR").unwrap_or_else(|| "ar".into()))
        };
        if self.msvc {
            command
                .arg("/nologo")
                .arg(format!("/OUT:{}", archive.display()))
                .arg(object);
        } else {
            command.arg("rcs").arg(archive).arg(object);
        }
        run(&mut command);
    }

    /// Links `objects` and `libraries` into `output`.
    pub(crate) fn link(&self, objects: &[&Path], libraries: &[&Path], output: &Path) {
        let mut command = Command::new(&self.program);
        if self.msvc {
            command
                .arg("/nologo")
                .arg(format!("/Fe:{}", output.display()));
            command.args(objects).args(libraries);
        } else {
            command.args(objects).args(libraries);
            command.arg("-o").arg(output);
        }
        run(&mut command);
    }
}
