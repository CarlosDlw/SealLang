//! End-to-end coverage for workspace queries, docs, and the API snapshot.

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
        "seal-workspace-test-{}-{}-{name}",
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

fn monorepo(name: &str) -> Fixture {
    let fixture = fixture(name);
    write(
        &fixture.root.join("seal.toml"),
        "[project]\nname = \"mono\"\nmembers = [\"crates/*\"]\n\
         default-members = [\"crates/alpha\"]\n\n\
         [tasks.ci]\ndepends-on = [\"hello\"]\nsteps = [{ program = \"echo\", args = [\"ci\"] }]\n\n\
         [tasks.hello]\nsteps = [{ program = \"echo\", args = [\"hello\"] }]\n",
    );
    write(
        &fixture.root.join("crates/alpha/seal.toml"),
        "[package]\nname = \"alpha\"\nversion = \"1.0.0\"\n",
    );
    write(
        &fixture.root.join("crates/alpha/src/lib.sl"),
        "//! Alpha helpers.\n\n\
         /// A point.\npublic struct Point {\n    /// The x coordinate.\n    public x: i32,\n    /// Internal state.\n    internal hidden: i32,\n}\n\n\
         /// Returns one.\npublic func one() -> i32 { 1 }\n",
    );
    write(
        &fixture.root.join("crates/beta/seal.toml"),
        "[package]\nname = \"beta\"\nversion = \"1.0.0\"\n\n\
         [dependencies]\nalpha = { path = \"../alpha\" }\n",
    );
    write(
        &fixture.root.join("crates/beta/src/lib.sl"),
        "use alpha::one;\n\n/// Returns two.\npublic func two() -> i32 { one() + 1 }\n",
    );
    fixture
}

#[test]
fn the_versioned_monorepo_fixture_supports_workspace_queries() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ztests/projects/monorepo");
    assert!(root.join("seal.toml").is_file());

    let graph = output(
        seal()
            .arg("graph")
            .arg("modules")
            .arg("--format")
            .arg("mermaid")
            .arg("--project")
            .arg(&root),
    );
    assert_success(&graph);
    assert!(String::from_utf8_lossy(&graph.stdout).contains("beta_3a_3aroot"));

    let output_dir = fixture("versioned-docs");
    let doc = output(
        seal()
            .arg("doc")
            .arg("--project")
            .arg(&root)
            .arg("-p")
            .arg("alpha")
            .arg("-o")
            .arg(&output_dir.root),
    );
    assert_success(&doc);
    let index = std::fs::read_to_string(output_dir.root.join("index.html")).expect("index");
    assert!(index.contains("Alpha contains the shared numeric helpers."));

    let api = output(
        seal()
            .arg("api")
            .arg("--project")
            .arg(&root)
            .arg("-p")
            .arg("alpha")
            .arg("--json"),
    );
    assert_success(&api);
    let api: serde_json::Value = serde_json::from_slice(&api.stdout).expect("API JSON");
    assert_eq!(api["package"], "alpha");

    let task = output(seal().arg("task").arg("ci").arg("--project").arg(&root));
    assert_success(&task);
}

#[test]
fn graph_json_contains_nodes_and_edges() {
    let fixture = monorepo("graph");
    let result = output(
        seal()
            .arg("graph")
            .arg("packages")
            .arg("--format")
            .arg("json")
            .arg("--project")
            .arg(&fixture.root),
    );
    assert_success(&result);
    let value: serde_json::Value =
        serde_json::from_slice(&result.stdout).expect("graph JSON parses");
    assert_eq!(value["version"], 1);
    assert_eq!(value["kind"], "packages");
    assert_eq!(value["nodes"].as_array().map(Vec::len), Some(2));
    assert_eq!(value["edges"].as_array().map(Vec::len), Some(1));
}

#[test]
fn doc_and_api_include_docs_layout_and_cfg() {
    let fixture = monorepo("docs");
    let result = output(
        seal()
            .arg("doc")
            .arg("--project")
            .arg(&fixture.root)
            .arg("-p")
            .arg("alpha")
            .arg("-o")
            .arg(fixture.root.join("target/doc")),
    );
    assert_success(&result);
    let index =
        std::fs::read_to_string(fixture.root.join("target/doc/index.html")).expect("index exists");
    assert!(index.contains("Alpha helpers."));
    assert!(index.contains("A point."));
    assert!(index.contains("The x coordinate."));
    assert!(!index.contains("Internal state."));
    assert!(!index.contains("hidden"));

    let api = output(
        seal()
            .arg("api")
            .arg("--project")
            .arg(&fixture.root)
            .arg("-p")
            .arg("alpha")
            .arg("--json"),
    );
    assert_success(&api);
    let value: serde_json::Value = serde_json::from_slice(&api.stdout).expect("API JSON parses");
    let point = value["items"]
        .as_array()
        .expect("items")
        .iter()
        .find(|item| item["name"] == "Point")
        .expect("Point is public");
    assert_eq!(point["docs"], "A point.");
    assert_eq!(point["layout"]["size"], 8);
    let fields = point["fields"].as_array().expect("fields");
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0]["docs"], "The x coordinate.");
}

#[test]
fn task_dependencies_run_before_the_task() {
    let fixture = monorepo("tasks");
    let result = output(
        seal()
            .arg("--verbose")
            .arg("task")
            .arg("ci")
            .arg("--project")
            .arg(&fixture.root),
    );
    assert_success(&result);
    let text = String::from_utf8_lossy(&result.stdout);
    let hello = text.find("hello").expect("dependency ran");
    let ci = text.find("ci").expect("task ran");
    assert!(hello < ci, "dependency output must come first: {text}");
}

#[test]
fn affected_maps_a_source_change_to_its_package() {
    let fixture = monorepo("affected");
    let graph = output(
        seal()
            .arg("graph")
            .arg("packages")
            .arg("--project")
            .arg(&fixture.root),
    );
    assert_success(&graph);

    let git = |args: &[&str]| {
        let result = output(
            Command::new("git")
                .arg("-C")
                .arg(&fixture.root)
                .args(["-c", "user.name=Seal", "-c", "user.email=seal@example.com"])
                .args(args),
        );
        assert_success(&result);
    };
    git(&["init", "-q"]);
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "fixture"]);

    write(
        &fixture.root.join("crates/beta/src/lib.sl"),
        "use alpha::one;\n\n/// Returns three.\npublic func two() -> i32 { one() + 2 }\n",
    );
    let result = output(
        seal()
            .arg("affected")
            .arg("--since")
            .arg("HEAD")
            .arg("--json")
            .arg("--project")
            .arg(&fixture.root),
    );
    assert_success(&result);
    let value: serde_json::Value = serde_json::from_slice(&result.stdout).expect("affected JSON");
    assert_eq!(value["packages"], serde_json::json!(["beta"]));
    let target = value["targets"].as_array().expect("targets");
    assert_eq!(target.len(), 1);
    assert_eq!(target[0]["package_name"], "beta");
}

#[test]
fn semver_check_accepts_a_compatible_baseline_file() {
    let fixture = monorepo("semver");
    let api = output(
        seal()
            .arg("api")
            .arg("--project")
            .arg(&fixture.root)
            .arg("-p")
            .arg("alpha")
            .arg("--json"),
    );
    assert_success(&api);
    let baseline = fixture.root.join("baseline.json");
    std::fs::write(&baseline, &api.stdout).expect("baseline");
    let result = output(
        seal()
            .arg("semver-check")
            .arg(&baseline)
            .arg("--project")
            .arg(&fixture.root)
            .arg("-p")
            .arg("alpha"),
    );
    assert_success(&result);
}
