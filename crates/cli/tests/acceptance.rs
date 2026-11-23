//! Acceptance tests for the documented project examples.
//!
//! Every example is copied into a temporary directory first, so the repository
//! never collects build artifacts and the examples stay usable as templates.

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

fn fixture(name: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "seal-acceptance-{}-{}-{name}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    Fixture { root }
}

fn seal() -> Command {
    Command::new(env!("CARGO_BIN_EXE_seal"))
}

fn output(command: &mut Command) -> Output {
    command.output().expect("the seal binary runs")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "status: {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create the destination directory");
    for entry in std::fs::read_dir(from).expect("read the example directory") {
        let entry = entry.expect("directory entry");
        let path = entry.path();
        let destination = to.join(entry.file_name());
        if path.is_dir() {
            copy_tree(&path, &destination);
        } else {
            std::fs::copy(&path, &destination).expect("copy an example file");
        }
    }
}

fn copy_example(name: &str) -> Fixture {
    let fixture = fixture(name);
    copy_tree(&examples().join(name), &fixture.root);
    fixture
}

#[test]
fn every_project_example_builds_and_runs() {
    for (name, expected) in [
        ("multi-module", 42),
        ("workspace", 42),
        ("features", 3),
        ("generator", 42),
        ("freestanding", 42),
        ("deprecation", 42),
    ] {
        let fixture = copy_example(name);
        let check = output(seal().arg("check").arg("--project").arg(&fixture.root));
        assert_success(&check);
        let build = output(seal().arg("build").arg("--project").arg(&fixture.root));
        assert_success(&build);
        let run = output(seal().arg("run").arg("--project").arg(&fixture.root));
        assert_eq!(
            run.status.code(),
            Some(expected),
            "{name}: stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        );
    }
}

#[test]
fn the_deprecation_example_warns_on_use() {
    let fixture = copy_example("deprecation");
    let check = output(seal().arg("check").arg("--project").arg(&fixture.root));
    assert_success(&check);
    let stderr = String::from_utf8_lossy(&check.stderr);
    assert!(
        stderr.contains("warning[deprecated]: `old` is deprecated since 0.1.0"),
        "{stderr}"
    );
    assert!(stderr.contains("this use is deprecated"), "{stderr}");
    assert!(stderr.contains("use current"), "{stderr}");
}

#[test]
fn metadata_and_doctor_describe_a_multi_file_project() {
    let fixture = copy_example("multi-module");
    let metadata = output(seal().arg("metadata").arg("--project").arg(&fixture.root));
    assert_success(&metadata);
    let metadata: serde_json::Value =
        serde_json::from_slice(&metadata.stdout).expect("metadata JSON");
    assert_eq!(metadata["version"], 1);
    assert_eq!(metadata["workspace"]["name"], "multi-module");
    assert_eq!(metadata["packages"][0]["name"], "multi-module");
    let modules = metadata["packages"][0]["modules"]
        .as_array()
        .expect("module list");
    assert!(
        modules.iter().any(|module| module == "app::math")
            && modules.iter().any(|module| module == "app::numbers")
            && modules.iter().any(|module| module == "root"),
        "{metadata}"
    );

    let fix = output(
        seal()
            .arg("doctor")
            .arg("--fix")
            .arg("--project")
            .arg(&fixture.root),
    );
    assert_success(&fix);
    let doctor = output(
        seal()
            .arg("doctor")
            .arg("--json")
            .arg("--project")
            .arg(&fixture.root),
    );
    assert_success(&doctor);
    let report: serde_json::Value = serde_json::from_slice(&doctor.stdout).expect("doctor JSON");
    assert!(report["version"].is_number(), "{report}");
    assert!(report["sandbox"].is_object(), "{report}");
}
