//! End-to-end standard-library cache and reproducibility coverage.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use seal_stdlib::Stdlib;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "seal-cli-stdlib-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("fixture root");
        Self { root }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("parent");
    }
    std::fs::write(path, contents).expect("write");
}

fn seal() -> Command {
    Command::new(env!("CARGO_BIN_EXE_seal"))
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

fn installed_stdlib() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../stdlib")
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("destination");
    for entry in std::fs::read_dir(from).expect("source directory") {
        let entry = entry.expect("directory entry");
        let destination = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&entry.path(), &destination);
        } else {
            std::fs::copy(entry.path(), &destination).expect("copy source");
        }
    }
}

fn project(fixture: &Fixture) {
    write(
        &fixture.path("seal.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nstdlib = \"core\"\n",
    );
    write(
        &fixture.path("src/main.sl"),
        "func main() -> i32 {\n  0\n}\n",
    );
}

fn build(fixture: &Fixture, stdlib: &Path) -> Output {
    seal()
        .arg("build")
        .arg("--project")
        .arg(&fixture.root)
        .env("SEAL_STDLIB", stdlib)
        .output()
        .expect("the seal binary runs")
}

fn refresh_digest(stdlib: &Path) {
    let error = Stdlib::load_from(stdlib)
        .expect_err("the edited tree no longer matches its digest")
        .to_string();
    let digest = error
        .split("has digest ")
        .nth(1)
        .and_then(|rest| rest.split(" but").next())
        .expect("the mismatch names the computed digest");
    let descriptor = stdlib.join("stdlib.toml");
    let text = std::fs::read_to_string(&descriptor).expect("descriptor");
    let updated = text
        .lines()
        .map(|line| {
            if line.starts_with("digest =") {
                format!("digest = \"{digest}\"")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(descriptor, format!("{updated}\n")).expect("updated descriptor");
}

#[test]
fn standard_library_content_and_not_mtime_drives_the_cache() {
    let fixture = Fixture::new("cache");
    project(&fixture);
    let stdlib = fixture.path("stdlib");
    copy_tree(&installed_stdlib(), &stdlib);

    let first = build(&fixture, &stdlib);
    assert_success(&first);

    let second = build(&fixture, &stdlib);
    assert_success(&second);
    let stdout = String::from_utf8_lossy(&second.stdout);
    for action in ["Cached frontend", "Cached codegen-cgu", "Cached link"] {
        assert!(
            stdout.contains(action),
            "the repeated build did not report `{action}`:\n{stdout}"
        );
    }

    let source = stdlib.join("core/src/lib.sl");
    let handle = std::fs::OpenOptions::new()
        .write(true)
        .open(&source)
        .expect("standard-library source");
    handle
        .set_modified(SystemTime::now() + Duration::from_secs(3600))
        .expect("touch the source");
    drop(handle);

    let third = build(&fixture, &stdlib);
    assert_success(&third);
    let stdout = String::from_utf8_lossy(&third.stdout);
    assert!(
        stdout.contains("Cached frontend"),
        "a timestamp-only change must keep the frontend cached:\n{stdout}"
    );

    let contents = std::fs::read_to_string(&source).expect("standard-library source");
    std::fs::write(&source, format!("{contents}\n// changed\n")).expect("edit the source");
    refresh_digest(&stdlib);

    let fourth = build(&fixture, &stdlib);
    assert_success(&fourth);
    let stdout = String::from_utf8_lossy(&fourth.stdout);
    assert!(
        !stdout.contains("Cached frontend"),
        "an edited standard-library source must invalidate the frontend:\n{stdout}"
    );
}

#[test]
fn stdlib_verify_reports_a_digest_mismatch() {
    let fixture = Fixture::new("verify");
    let stdlib = fixture.path("stdlib");
    copy_tree(&installed_stdlib(), &stdlib);
    let source = stdlib.join("core/src/lib.sl");
    let contents = std::fs::read_to_string(&source).expect("standard-library source");
    std::fs::write(&source, format!("{contents}\n// edited\n")).expect("edit the source");

    let verify = seal()
        .arg("stdlib")
        .arg("verify")
        .env("SEAL_STDLIB", &stdlib)
        .output()
        .expect("the seal binary runs");
    assert!(!verify.status.success());
    assert!(
        String::from_utf8_lossy(&verify.stderr).contains("E8140"),
        "stderr: {}",
        String::from_utf8_lossy(&verify.stderr)
    );
}

#[test]
fn test_and_bench_use_the_package_standard_library_mode() {
    let fixture = Fixture::new("runner");
    write(
        &fixture.path("seal.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nstdlib = \"core\"\n",
    );
    write(
        &fixture.path("src/lib.sl"),
        "public func value() -> i32 { 7 }\n\ntest func value_is_seven() { assert(value() == 7); }\n",
    );
    write(
        &fixture.path("benches/work.sl"),
        "bench func value_loop() {\n  let mut index = 0;\n  while index < 10 {\n    assert(index < 10);\n    index = index + 1;\n  }\n}\n",
    );
    let stdlib = fixture.path("stdlib");
    copy_tree(&installed_stdlib(), &stdlib);

    let tests = seal()
        .arg("test")
        .arg("--project")
        .arg(&fixture.root)
        .env("SEAL_STDLIB", &stdlib)
        .output()
        .expect("the seal binary runs");
    assert_success(&tests);
    assert!(String::from_utf8_lossy(&tests.stdout).contains("value_is_seven"));

    let benches = seal()
        .arg("bench")
        .arg("--project")
        .arg(&fixture.root)
        .arg("--iterations")
        .arg("1")
        .arg("--samples")
        .arg("1")
        .env("SEAL_STDLIB", &stdlib)
        .output()
        .expect("the seal binary runs");
    assert_success(&benches);
    assert!(String::from_utf8_lossy(&benches.stdout).contains("value_loop"));

    let docs = seal()
        .arg("doc")
        .arg("--project")
        .arg(&fixture.root)
        .arg("-o")
        .arg(fixture.path("docs"))
        .env("SEAL_STDLIB", &stdlib)
        .output()
        .expect("the seal binary runs");
    assert_success(&docs);
    assert!(fixture.path("docs/index.html").is_file());

    let api = seal()
        .arg("api")
        .arg("--project")
        .arg(&fixture.root)
        .arg("--json")
        .env("SEAL_STDLIB", &stdlib)
        .output()
        .expect("the seal binary runs");
    assert_success(&api);
    let api: serde_json::Value = serde_json::from_slice(&api.stdout).expect("API JSON");
    assert_eq!(api["package"], "demo");
    let items = api["items"].as_array().expect("API items");
    assert_eq!(
        items.len(),
        1,
        "the stdlib must not enter the user API: {api}"
    );
    assert_eq!(items[0]["name"], "value");
    assert_eq!(api["modules"].as_array().map(Vec::len), Some(1));
}

#[test]
fn the_standard_library_is_not_a_project_dependency() {
    let fixture = Fixture::new("metadata");
    project(&fixture);
    let stdlib = fixture.path("stdlib");
    copy_tree(&installed_stdlib(), &stdlib);
    assert_success(&build(&fixture, &stdlib));

    let metadata = seal()
        .arg("metadata")
        .arg("--project")
        .arg(&fixture.root)
        .output()
        .expect("the seal binary runs");
    assert_success(&metadata);
    let metadata: serde_json::Value =
        serde_json::from_slice(&metadata.stdout).expect("metadata is JSON");
    let packages = metadata["packages"]
        .as_array()
        .expect("metadata lists packages");
    assert_eq!(packages.len(), 1);
    assert_eq!(packages[0]["name"], "demo");
    assert_eq!(
        packages[0]["dependencies"].as_array().map(Vec::len),
        Some(0)
    );

    let tree = seal()
        .arg("tree")
        .arg("--project")
        .arg(&fixture.root)
        .output()
        .expect("the seal binary runs");
    assert_success(&tree);
    let tree = String::from_utf8_lossy(&tree.stdout);
    assert!(!tree.contains("core 0.1.0"), "tree:\n{tree}");
    assert!(!tree.contains("std 0.1.0"), "tree:\n{tree}");

    let lock = std::fs::read_to_string(fixture.path("seal.lock")).expect("lockfile");
    assert!(!lock.contains("name = \"core\""), "lockfile:\n{lock}");
    assert!(!lock.contains("name = \"std\""), "lockfile:\n{lock}");
}

#[test]
fn relocating_the_standard_library_keeps_artifacts_identical() {
    let fixture = Fixture::new("relocated");
    project(&fixture);
    let first_stdlib = fixture.path("stdlib-a");
    let second_stdlib = fixture.path("stdlib-b");
    copy_tree(&installed_stdlib(), &first_stdlib);
    copy_tree(&installed_stdlib(), &second_stdlib);

    let first = build(&fixture, &first_stdlib);
    assert_success(&first);
    let artifact = fixture.path("seal/target/dev/demo");
    let expected = std::fs::read(&artifact).expect("first artifact");

    std::fs::remove_dir_all(fixture.path("seal")).expect("clean build state");
    let second = build(&fixture, &second_stdlib);
    assert_success(&second);
    let actual = std::fs::read(&artifact).expect("second artifact");
    assert_eq!(
        actual, expected,
        "moving the standard library must not change the artifact"
    );

    let other = Fixture::new("relocated-other");
    project(&other);
    let third = build(&other, &second_stdlib);
    assert_success(&third);
    let elsewhere = std::fs::read(other.path("seal/target/dev/demo"))
        .expect("artifact in another project directory");
    assert_eq!(
        elsewhere, expected,
        "two project directories must produce identical artifacts"
    );
}
