//! End-to-end tests for the `seal` binary.

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
        "seal-cli-test-{}-{}-{name}",
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

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("parent");
    }
    std::fs::write(path, contents).expect("write");
}

#[test]
fn a_new_binary_project_builds_checks_and_runs() {
    let fixture = fixture("bin");
    let created = output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    );
    assert_success(&created);
    let build = output(seal().arg("build").arg("--project").arg(&fixture.root));
    assert_success(&build);
    let artifact = std::fs::read_dir(fixture.root.join("seal/target/dev"))
        .expect("target directory")
        .filter_map(Result::ok)
        .find(|entry| entry.path().extension().is_none())
        .expect("an executable artifact");
    assert!(artifact.path().is_file());

    let check = output(seal().arg("check").arg("--project").arg(&fixture.root));
    assert_success(&check);

    write(
        &fixture.root.join("src/main.sl"),
        "use app::util::value;\n\nfunc main() -> i32 {\n  value() + 7\n}\n",
    );
    let run = output(seal().arg("run").arg("--project").arg(&fixture.root));
    assert_eq!(run.status.code(), Some(7));
}

#[test]
fn a_library_project_builds_without_a_main() {
    let fixture = fixture("lib");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--template")
            .arg("lib")
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    let build = output(seal().arg("build").arg("--project").arg(&fixture.root));
    assert_success(&build);
    let objects: Vec<_> = std::fs::read_dir(fixture.root.join("seal/target/dev"))
        .expect("target directory")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "o")
        })
        .collect();
    assert_eq!(objects.len(), 1);
}

#[test]
fn a_workspace_builds_every_member() {
    let fixture = fixture("workspace");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--template")
            .arg("workspace")
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    let build = output(
        seal()
            .arg("build")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--workspace"),
    );
    assert_success(&build);
    assert!(fixture.root.join("seal/target/dev/app").is_file());
    assert!(fixture.root.join("seal/target/dev/libgreet.o").is_file());
}

#[test]
fn all_targets_builds_tests_benches_and_examples() {
    let fixture = fixture("all-targets");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    write(
        &fixture.root.join("seal.toml"),
        "[project]\nname = \"all-targets\"\n\n[package]\nname = \"all-targets\"\nversion = \"0.1.0\"\n\n\
         [targets.share]\nkind = \"lib\"\n\n\
         [targets.main]\nkind = \"bin\"\nentry = \"root::main\"\n\n\
         [targets.checks]\nkind = \"test\"\n\n\
         [targets.measure]\nkind = \"bench\"\n\n\
         [targets.demo]\nkind = \"example\"\nentry = \"root::main\"\n",
    );
    write(
        &fixture.root.join("src/lib.sl"),
        "public func answer() -> i32 { 0 }\n",
    );
    write(
        &fixture.root.join("tests/checks.sl"),
        "func main() -> i32 { 0 }\n",
    );
    write(
        &fixture.root.join("benches/measure.sl"),
        "func main() -> i32 { 0 }\n",
    );
    write(
        &fixture.root.join("examples/demo.sl"),
        "func main() -> i32 { 0 }\n",
    );

    let build = output(
        seal()
            .arg("build")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--all-targets"),
    );
    assert_success(&build);
    for artifact in ["main", "checks", "measure", "demo", "libshare.o"] {
        assert!(
            fixture
                .root
                .join("seal/target/dev")
                .join(artifact)
                .is_file(),
            "--all-targets writes {artifact}"
        );
    }
}

#[test]
fn quiet_suppresses_status_and_color_is_explicit() {
    let fixture = fixture("output-policy");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));

    let quiet = output(
        seal()
            .arg("build")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--quiet"),
    );
    assert_success(&quiet);
    assert!(
        quiet.stdout.is_empty(),
        "--quiet writes no status to stdout"
    );

    let colored = output(
        seal()
            .arg("build")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--color")
            .arg("always"),
    );
    assert_success(&colored);
    assert!(
        String::from_utf8_lossy(&colored.stdout).contains("\u{1b}[1m"),
        "--color=always emits ANSI even when stdout is redirected"
    );
}

#[test]
fn metadata_is_parseable_and_lists_modules_and_targets() {
    let fixture = fixture("metadata");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    let metadata = output(
        seal()
            .arg("metadata")
            .arg("--json")
            .arg("--project")
            .arg(&fixture.root),
    );
    assert_success(&metadata);
    let value: serde_json::Value =
        serde_json::from_slice(&metadata.stdout).expect("metadata is JSON");
    assert_eq!(value["version"], 1);
    let package = &value["packages"][0];
    assert!(
        package["name"]
            .as_str()
            .is_some_and(|name| name.starts_with("seal-cli-test"))
    );
    assert!(
        package["targets"]
            .as_array()
            .is_some_and(|targets| !targets.is_empty())
    );
    assert!(
        package["modules"]
            .as_array()
            .is_some_and(|modules| modules.iter().any(|module| module == "app::util"))
    );
}

#[test]
fn two_binaries_make_run_ambiguous_until_one_is_selected() {
    let fixture = fixture("bins");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    write(
        &fixture.root.join("seal.toml"),
        "[project]\nname = \"seal-cli-test\"\n\n[package]\nname = \"seal-cli-test\"\nversion = \"0.1.0\"\n\n\
         [targets.first]\nkind = \"bin\"\nentry = \"root::main\"\n\n\
         [targets.second]\nkind = \"bin\"\nentry = \"root::main\"\n",
    );
    let run = output(seal().arg("run").arg("--project").arg(&fixture.root));
    assert_eq!(run.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&run.stderr).contains("E8103"));
    let selected = output(
        seal()
            .arg("run")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--bin")
            .arg("first"),
    );
    assert_success(&selected);
}

#[test]
fn an_entry_that_names_no_function_is_e2087() {
    let fixture = fixture("entry");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    write(
        &fixture.root.join("seal.toml"),
        "[project]\nname = \"seal-cli-test\"\n\n[package]\nname = \"seal-cli-test\"\nversion = \"0.1.0\"\n\n\
         [targets.main]\nkind = \"bin\"\nentry = \"app::missing\"\n",
    );
    let build = output(seal().arg("build").arg("--project").arg(&fixture.root));
    assert_eq!(build.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&build.stderr).contains("E2087"));
}

#[test]
fn a_target_without_an_entry_is_e2087() {
    let fixture = fixture("missing-entry");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    write(
        &fixture.root.join("seal.toml"),
        "[project]\nname = \"missing-entry\"\n\n[package]\nname = \"missing-entry\"\nversion = \"0.1.0\"\n\n\
         [targets.main]\nkind = \"bin\"\n",
    );
    let build = output(seal().arg("build").arg("--project").arg(&fixture.root));
    assert_eq!(build.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&build.stderr).contains("E2087"));
}

#[test]
fn a_missing_manifest_points_at_seal_init() {
    let fixture = fixture("no-manifest");
    std::fs::create_dir_all(&fixture.root).expect("root");
    let build = output(seal().arg("build").arg("--project").arg(&fixture.root));
    assert_eq!(build.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&build.stderr).contains("seal init"));
}

#[test]
fn paths_with_spaces_and_unicode_work() {
    let fixture = fixture("spaces");
    let root = fixture.root.join("projeto com espaço");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&root)
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    let build = output(seal().arg("build").arg("--project").arg(&root));
    assert_success(&build);
}
