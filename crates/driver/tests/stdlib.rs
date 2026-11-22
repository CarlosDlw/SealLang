//! Standard-library injection tests for the low-level driver.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn temporary(name: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "seal-driver-stdlib-{}-{}-{name}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the fixture root is writable");
    Fixture { root }
}

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the fixture directory is writable");
    }
    std::fs::write(path, contents).expect("the fixture file is writable");
}

fn run(arguments: &[&str], environment: Option<(&str, &Path)>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sealc"));
    command.args(arguments);
    if let Some((name, value)) = environment {
        command.env(name, value);
    }
    command.output().expect("sealc runs")
}

fn stdlib_fixture(root: &Path, core_source: &str) {
    write(
        root,
        "stdlib.toml",
        "format = 1\nlanguage = \"0.1\"\nversion = \"0.1.0\"\npackages = [\"core\", \"std\"]\ndigest = \"0000000000000000000000000000000000000000000000000000000000000000\"\n\n[prelude]\nstd = \"std::prelude\"\ncore = \"core::prelude\"\n",
    );
    write(
        root,
        "core/seal.toml",
        "[package]\nname = \"core\"\nversion = \"0.1.0\"\nstdlib = \"none\"\nimplicit-prelude = false\n\n[package.api]\nexpose = [\"root\", \"prelude\"]\n",
    );
    write(root, "core/src/lib.sl", core_source);
    write(root, "core/src/prelude.sl", "module prelude;\n");
    write(
        root,
        "std/seal.toml",
        "[package]\nname = \"std\"\nversion = \"0.1.0\"\nstdlib = \"none\"\nimplicit-prelude = false\n\n[dependencies]\ncore = { path = \"../core\" }\n\n[package.api]\nexpose = [\"root\", \"prelude\"]\n",
    );
    write(root, "std/src/lib.sl", "// std\n");
    write(root, "std/src/prelude.sl", "module prelude;\n");
}

/// Reads the computed digest from the loader's mismatch diagnostic, then writes
/// it into the descriptor.
fn seal_digest(root: &Path) {
    let error = seal_stdlib::Stdlib::load_from(root).expect_err("the placeholder digest differs");
    let digest = error
        .message()
        .split_whitespace()
        .find(|word| word.len() == 64 && word.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .expect("the diagnostic names the computed digest")
        .to_owned();
    write(
        root,
        "stdlib.toml",
        &format!(
            "format = 1\nlanguage = \"0.1\"\nversion = \"0.1.0\"\npackages = [\"core\", \"std\"]\ndigest = \"{digest}\"\n\n[prelude]\nstd = \"std::prelude\"\ncore = \"core::prelude\"\n"
        ),
    );
}

#[test]
fn no_stdlib_compiles_without_an_installation() {
    let fixture = temporary("no-stdlib");
    let source = fixture.root.join("main.sl");
    std::fs::write(&source, "func main() -> i32 { 0 }\n").expect("source");
    let output = run(
        &["check", "--no-stdlib", &source.display().to_string()],
        Some(("SEAL_STDLIB", &fixture.root.join("missing"))),
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn core_only_uses_the_core_prelude() {
    let fixture = temporary("core-only");
    let source = fixture.root.join("main.sl");
    std::fs::write(&source, "func main() -> i32 { 0 }\n").expect("source");
    let output = run(
        &["check", "--core-only", &source.display().to_string()],
        None,
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn std_and_core_paths_name_the_same_types() {
    let fixture = temporary("identity");
    let source = fixture.root.join("main.sl");
    std::fs::write(
        &source,
        r#"use core::result::Result;
use std::option::Option;

func core_option(value: Option<i32>) -> core::option::Option<i32> {
  value
}

func std_prelude(value: core::prelude::Option<i32>) -> std::prelude::Option<i32> {
  value
}

func aliases(value: std::option::Option<i32>) -> core::option::Option<i32> {
  value
}

func results(value: std::result::Result<i32, i32>) -> core::result::Result<i32, i32> {
  value
}

func main() -> i32 {
  let value = Option::Some(41);
  let from_core = core_option(value);
  let from_std = std_prelude(from_core);
  let alias = aliases(from_std);
  let number = match alias {
    Option::Some(inner) => inner,
    Option::None => 0,
  };
  let ok = Result::<i32, i32>::Ok(number);
  let result = results(ok);
  match result {
    Result::Ok(value) => value - 41,
    Result::Err(_) => 1,
  }
}
"#,
    )
    .expect("source");
    let output = run(&["check", &source.display().to_string()], None);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_core_prelude_exposes_option_and_result() {
    let fixture = temporary("core-prelude");
    let source = fixture.root.join("main.sl");
    std::fs::write(
        &source,
        "func main() -> i32 {\n  let some = Option::Some(1);\n  let ok = Result::<i32, i32>::Ok(2);\n  match some {\n    Option::Some(value) => value,\n    Option::None => 0,\n  }\n}\n",
    )
    .expect("source");
    let output = run(
        &["check", "--core-only", &source.display().to_string()],
        None,
    );
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn alloc_is_exposed_only_by_the_hosted_standard_library() {
    let fixture = temporary("alloc-surface");
    let source = fixture.root.join("main.sl");
    std::fs::write(
        &source,
        "use std::alloc::Layout;\n\nfunc main() -> usize {\n  Layout::new::<u8>().size()\n}\n",
    )
    .expect("source");

    let hosted = run(&["check", &source.display().to_string()], None);
    assert!(
        hosted.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&hosted.stderr)
    );

    let core = run(
        &["check", "--core-only", &source.display().to_string()],
        None,
    );
    assert_eq!(core.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&core.stderr);
    assert!(stderr.contains("E2073"), "stderr: {stderr}");
    assert!(stderr.contains("std"), "stderr: {stderr}");
    assert!(!stderr.contains("E2070"), "cascading error: {stderr}");

    let none = run(
        &["check", "--no-stdlib", &source.display().to_string()],
        Some(("SEAL_STDLIB", &fixture.root.join("missing"))),
    );
    assert_eq!(none.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&none.stderr);
    assert!(stderr.contains("E2073"), "stderr: {stderr}");
    assert!(stderr.contains("std"), "stderr: {stderr}");
    assert!(!stderr.contains("E2070"), "cascading error: {stderr}");
}

#[test]
fn allocation_wrappers_require_unsafe_and_runtime_symbols_stay_private() {
    let fixture = temporary("alloc-safety");
    let unsafe_source = fixture.root.join("unsafe.sl");
    std::fs::write(
        &unsafe_source,
        "use std::alloc::alloc;\nuse std::alloc::Layout;\n\nfunc main() {\n  let layout = Layout::new::<u8>();\n  alloc(layout);\n}\n",
    )
    .expect("unsafe source");
    let unsafe_output = run(&["check", &unsafe_source.display().to_string()], None);
    assert_eq!(unsafe_output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&unsafe_output.stderr);
    assert!(stderr.contains("E3070"), "stderr: {stderr}");
    assert!(stderr.contains("unsafe"), "stderr: {stderr}");

    let private_source = fixture.root.join("private.sl");
    std::fs::write(
        &private_source,
        "use std::alloc::seal_alloc;\n\nfunc main() {}\n",
    )
    .expect("private source");
    let private_output = run(&["check", &private_source.display().to_string()], None);
    assert_eq!(private_output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&private_output.stderr);
    assert!(stderr.contains("E2073"), "stderr: {stderr}");
    assert!(stderr.contains("seal_alloc"), "stderr: {stderr}");
}

#[test]
fn no_stdlib_rejects_string_methods_without_crashing() {
    let fixture = temporary("no-stdlib-string");
    let source = fixture.root.join("main.sl");
    std::fs::write(
        &source,
        "func main() -> i32 {\n  let text: *str = \"a\";\n  let mut chars = text.chars();\n  if chars.has_next() { 1 } else { 0 }\n}\n",
    )
    .expect("source");
    let output = run(
        &["check", "--no-stdlib", &source.display().to_string()],
        Some(("SEAL_STDLIB", &fixture.root.join("missing"))),
    );
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E3027"), "stderr: {stderr}");
    assert!(stderr.contains("chars"), "stderr: {stderr}");
}

#[test]
fn no_stdlib_does_not_leak_prelude_names() {
    let fixture = temporary("none-prelude");
    let source = fixture.root.join("main.sl");
    std::fs::write(
        &source,
        "func main() -> i32 {\n  let value = Option::Some(1);\n  match value {\n    Option::Some(inner) => inner,\n    Option::None => 0,\n  }\n}\n",
    )
    .expect("source");
    let output = run(
        &["check", "--no-stdlib", &source.display().to_string()],
        None,
    );
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E2002"), "stderr: {stderr}");
    assert!(stderr.contains("Option"), "stderr: {stderr}");
}

#[test]
fn option_moves_and_is_not_copy() {
    let fixture = temporary("move");
    let source = fixture.root.join("main.sl");
    std::fs::write(
        &source,
        "func consume(value: Option<i32>) -> i32 {\n  match value {\n    Option::Some(inner) => inner,\n    Option::None => 0,\n  }\n}\n\nfunc main() -> i32 {\n  let value = Option::Some(1);\n  let first = consume(value);\n  let second = consume(value);\n  first + second\n}\n",
    )
    .expect("source");
    let output = run(&["check", &source.display().to_string()], None);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E5005"), "stderr: {stderr}");
}

#[test]
fn a_plan_with_another_digest_is_e8141() {
    let fixture = temporary("mismatch");
    let source = fixture.root.join("main.sl");
    std::fs::write(&source, "func main() -> i32 { 0 }\n").expect("source");
    let plan = fixture.root.join("plan.json");
    std::fs::write(
        &plan,
        format!(
            "{{\"version\":2,\"target\":\"{}\",\"profile\":{{}},\"mode\":\"check\",\"packages\":[{{\"name\":\"app\",\"version\":\"0.1.0\",\"instance\":0,\"root\":\"{}\",\"files\":[{{\"path\":\"{}\",\"primary\":true}}],\"stdlib\":\"std\"}}],\"primary\":0,\"stdlib\":{{\"language\":\"0.1\",\"version\":\"0.1.0\",\"digest\":\"{}\"}}}}",
            seal_target::host_triple(),
            fixture.root.display(),
            source.display(),
            "0".repeat(64)
        ),
    )
    .expect("plan");
    let output = run(&["check", "--plan-file", &plan.display().to_string()], None);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E8141"), "stderr: {stderr}");
}

#[test]
fn a_standard_library_diagnostic_uses_the_stdlib_placeholder() {
    let fixture = temporary("diagnostic");
    let stdlib = fixture.root.join("stdlib");
    stdlib_fixture(&stdlib, "func broken( {\n");
    seal_digest(&stdlib);
    let source = fixture.root.join("main.sl");
    std::fs::write(&source, "func main() -> i32 { 0 }\n").expect("source");
    let output = run(
        &["check", &source.display().to_string()],
        Some(("SEAL_STDLIB", &stdlib)),
    );
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("<stdlib>/core/src/lib.sl"),
        "stderr: {stderr}"
    );
    assert!(
        !stderr.contains(&stdlib.display().to_string()),
        "the absolute stdlib root leaked: {stderr}"
    );
}
