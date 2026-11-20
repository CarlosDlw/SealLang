//! End-to-end coverage for `--plan`: several files compiled as one package.

use std::path::{Path, PathBuf};
use std::process::Command;

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("seal-plan-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&path).expect("the scratch directory is created");
        Self(path)
    }

    fn write(&self, name: &str, source: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, source).expect("the source file is written");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn sealc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_sealc"))
}

fn package_files(scratch: &Scratch) -> Vec<PathBuf> {
    vec![
        scratch.write(
            "main.sl",
            "use app::util::{double, extra};\n\
             func main() -> i32 { double(20) + extra(1) }",
        ),
        scratch.write(
            "util.sl",
            "module app::util;\n\
             internal func double(value: i32) -> i32 { value * 2 }",
        ),
        scratch.write(
            "fragment.sl",
            "module app::util;\n\
             internal func extra(value: i32) -> i32 { value + 1 }",
        ),
    ]
}

#[test]
fn a_multi_file_package_checks_through_the_driver() {
    let scratch = Scratch::new("check");
    let files = package_files(&scratch);
    let output = sealc()
        .args(["check", "--plan", "--expose", "app::util"])
        .args(&files)
        .output()
        .expect("the compiler runs");
    assert!(
        output.status.success(),
        "the package failed to check: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stderr.is_empty(),
        "the package reported a diagnostic: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn check_fails(scratch: &Scratch, expected: &str) {
    let files = [scratch.0.join("main.sl"), scratch.0.join("app.sl")];
    let output = sealc()
        .args(["check", "--plan"])
        .args(&files)
        .output()
        .expect("the compiler runs");
    assert!(!output.status.success(), "the violation was not reported");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(expected),
        "expected `{expected}` in: {stderr}"
    );
}

#[test]
fn a_private_field_cannot_be_read_from_another_module() {
    let scratch = Scratch::new("private-field-read");
    scratch.write(
        "main.sl",
        "use app::Widget;\nfunc main() -> i32 { let value = Widget { hidden: 1 }; value.hidden }",
    );
    scratch.write(
        "app.sl",
        "module app;\ninternal struct Widget { private hidden: i32 }",
    );
    check_fails(&scratch, "E2072");
}

#[test]
fn a_private_field_cannot_be_initialized_from_another_module() {
    let scratch = Scratch::new("private-field-init");
    scratch.write(
        "main.sl",
        "use app::Widget;\nfunc main() -> i32 { let value = Widget { hidden: 1 }; 0 }",
    );
    scratch.write(
        "app.sl",
        "module app;\ninternal struct Widget { private hidden: i32 }",
    );
    check_fails(&scratch, "E2072");
}

#[test]
fn a_private_method_cannot_be_called_from_another_module() {
    let scratch = Scratch::new("private-method");
    scratch.write(
        "main.sl",
        "use app::Widget;\nfunc main() -> i32 { let value = Widget { x: 1 }; value.secret() }",
    );
    scratch.write(
        "app.sl",
        "module app;\n\
         internal struct Widget { internal x: i32 }\n\
         impl Widget { private func secret(self: *Widget) -> i32 { (*self).x } }",
    );
    check_fails(&scratch, "E2072");
}

#[test]
fn a_multi_file_package_builds_and_runs() {
    let scratch = Scratch::new("build");
    let files = package_files(&scratch);
    let output = sealc()
        .current_dir(&scratch.0)
        .args(["build", "--plan", "--expose", "app::util", "-o", "example"])
        .args(&files)
        .output()
        .expect("the compiler runs");
    assert!(
        output.status.success(),
        "the package failed to build: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let executable = scratch.0.join("example");
    assert!(Path::new(&executable).exists(), "the artifact was written");
    let status = Command::new(&executable)
        .status()
        .expect("the executable runs");
    assert_eq!(status.code(), Some(42));
}
