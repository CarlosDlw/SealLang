//! Tests for the versioned `--plan-file` path.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use seal_plan::{BuildMode, CompilePlan, PlanFile, PlanPackage, PlanProfile};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn fixture(source: &str) -> (Fixture, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "seal-plan-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("root");
    let source_path = root.join("main.sl");
    std::fs::write(&source_path, source).expect("source");
    (Fixture { root }, source_path)
}

fn plan(
    source: &Path,
    entry: Option<&str>,
    mode: BuildMode,
    output: Option<PathBuf>,
) -> CompilePlan {
    CompilePlan {
        version: seal_plan::FORMAT_VERSION,
        target: seal_target::host_triple().as_str().to_owned(),
        profile: PlanProfile::default(),
        mode,
        packages: vec![PlanPackage {
            name: "test".to_owned(),
            version: "0.1.0".to_owned(),
            instance: 0,
            root: source.parent().expect("source parent").to_path_buf(),
            files: vec![PlanFile {
                path: source.to_path_buf(),
                primary: true,
            }],
            exposed: vec!["root".to_owned()],
            dependencies: Vec::new(),
            cfg: seal_plan::CfgOptions::default(),
        }],
        primary: 0,
        entry: entry.map(str::to_owned),
        metadata: seal_plan::PlanMetadata::default(),
        output,
    }
}

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sealc"))
        .args(arguments)
        .output()
        .expect("sealc runs")
}

fn write_plan(root: &Path, plan: &CompilePlan) -> PathBuf {
    let path = root.join("plan.json");
    plan.write(&path).expect("plan writes");
    path
}

#[test]
fn check_reads_a_plan_file() {
    let (fixture, source) = fixture("func main() -> i32 { 3 }\n");
    let plan = plan(&source, Some("root::main"), BuildMode::Check, None);
    let path = write_plan(&fixture.root, &plan);
    let output = run(&["check", "--plan-file", &path.display().to_string()]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn an_explicit_entry_builds_and_runs() {
    let (fixture, source) = fixture("module app;\nfunc main() -> i32 { 3 }\n");
    let output_path = fixture.root.join("program");
    let plan = plan(
        &source,
        Some("app::main"),
        BuildMode::Build,
        Some(output_path.clone()),
    );
    let path = write_plan(&fixture.root, &plan);
    let output = run(&["build", "--plan-file", &path.display().to_string()]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let status = Command::new(&output_path).status().expect("program runs");
    assert_eq!(status.code(), Some(3));
}

#[test]
fn two_packages_with_the_same_function_name_do_not_collide() {
    let (fixture, _source) = fixture("");
    let greet = fixture.root.join("greet.sl");
    let app = fixture.root.join("app.sl");
    std::fs::write(&greet, "public func answer() -> i32 { 2 }\n").expect("greet");
    std::fs::write(
        &app,
        "use greet::answer as dependency_answer;\n\nfunc answer() -> i32 { 3 }\n\nfunc main() -> i32 {\n  answer() + dependency_answer()\n}\n",
    )
    .expect("app");
    let output_path = fixture.root.join("two-packages");
    let plan = CompilePlan {
        version: seal_plan::FORMAT_VERSION,
        target: seal_target::host_triple().as_str().to_owned(),
        profile: PlanProfile::default(),
        mode: BuildMode::Build,
        packages: vec![
            PlanPackage {
                name: "greet".to_owned(),
                version: "0.1.0".to_owned(),
                instance: 0,
                root: fixture.root.clone(),
                files: vec![PlanFile {
                    path: greet,
                    primary: true,
                }],
                exposed: vec!["root".to_owned()],
                dependencies: Vec::new(),
                cfg: seal_plan::CfgOptions::default(),
            },
            PlanPackage {
                name: "app".to_owned(),
                version: "0.1.0".to_owned(),
                instance: 0,
                root: fixture.root.clone(),
                files: vec![PlanFile {
                    path: app,
                    primary: true,
                }],
                exposed: vec!["root".to_owned()],
                dependencies: vec![seal_plan::PlanDependency {
                    alias: "greet".to_owned(),
                    package: 0,
                }],
                cfg: seal_plan::CfgOptions::default(),
            },
        ],
        primary: 1,
        entry: Some("root::main".to_owned()),
        metadata: seal_plan::PlanMetadata::default(),
        output: Some(output_path.clone()),
    };
    let path = write_plan(&fixture.root, &plan);
    let output = run(&["build", "--plan-file", &path.display().to_string()]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let status = Command::new(&output_path).status().expect("program runs");
    assert_eq!(status.code(), Some(5));
}

#[test]
fn malformed_json_is_e8001_without_a_panic() {
    let (fixture, _source) = fixture("func main() -> i32 { 0 }\n");
    let path = fixture.root.join("bad.json");
    std::fs::write(&path, "{").expect("bad plan");
    let output = run(&["check", "--plan-file", &path.display().to_string()]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E8001"), "stderr: {stderr}");
    assert!(!stderr.contains("panicked"));
}

#[test]
fn a_structurally_invalid_plan_is_e8010() {
    let (fixture, _source) = fixture("func main() -> i32 { 0 }\n");
    let path = fixture.root.join("bad.json");
    std::fs::write(
        &path,
        "{\"version\":1,\"target\":\"x86_64-unknown-linux-gnu\",\"profile\":{},\
         \"mode\":\"check\",\"packages\":[],\"primary\":0}",
    )
    .expect("bad plan");
    let output = run(&["check", "--plan-file", &path.display().to_string()]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("E8010"));
}

#[test]
fn plan_mode_emits_an_api_snapshot() {
    let (fixture, source) = fixture("func main() -> i32 { 0 }\n");
    let plan = plan(&source, Some("root::main"), BuildMode::Check, None);
    let path = write_plan(&fixture.root, &plan);
    let api = fixture.root.join("api.json");
    let output = run(&[
        "build",
        "--emit=api",
        "--plan-file",
        &path.display().to_string(),
        "-o",
        &api.display().to_string(),
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let snapshot: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&api).expect("api file")).expect("api json");
    assert_eq!(snapshot["version"], 1);
    assert!(
        snapshot["modules"]
            .as_array()
            .is_some_and(|modules| !modules.is_empty())
    );
}
