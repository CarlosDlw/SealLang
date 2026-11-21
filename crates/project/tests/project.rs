//! Project discovery, graph, target, and plan tests.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use seal_manifest::TargetKind;
use seal_plan::BuildMode;
use seal_project::{PlanOptions, Project};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn fixture(files: &[(&str, &str)]) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "seal-project-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temporary root");
    for (path, contents) in files {
        let path = root.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent");
        }
        std::fs::write(path, contents).expect("fixture file");
    }
    Fixture { root }
}

fn plan_options() -> PlanOptions {
    PlanOptions::default()
}

#[test]
fn a_single_package_discovers_its_default_binary() {
    let fixture = fixture(&[
        (
            "seal.toml",
            "[project]\nname = \"demo\"\n\n[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        ),
        ("src/main.sl", "func main() -> i32 { 0 }\n"),
    ]);
    let project = Project::load(&fixture.root).expect("project");
    assert_eq!(project.name, "demo");
    let package = project.package(0);
    assert_eq!(package.targets.len(), 1);
    assert_eq!(package.targets[0].kind, TargetKind::Bin);
    assert_eq!(package.targets[0].entry.as_deref(), Some("root::main"));
    let plan = project
        .compile_plan(0, &package.targets[0].name, &plan_options())
        .expect("plan");
    assert_eq!(plan.entry.as_deref(), Some("root::main"));
    assert_eq!(plan.primary, 0);
}

#[test]
fn a_workspace_orders_path_dependencies_first() {
    let fixture = fixture(&[
        (
            "seal.toml",
            "[project]\nname = \"ws\"\nmembers = [\"crates/*\"]\n",
        ),
        (
            "crates/app/seal.toml",
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
             [dependencies]\ngreet = { path = \"../greet\" }\n",
        ),
        ("crates/app/src/main.sl", "func main() -> i32 { 0 }\n"),
        (
            "crates/greet/seal.toml",
            "[package]\nname = \"greet\"\nversion = \"0.1.0\"\n",
        ),
        (
            "crates/greet/src/lib.sl",
            "public func answer() -> i32 { 42 }\n",
        ),
    ]);
    let project = Project::load(&fixture.root).expect("project");
    assert_eq!(project.packages.len(), 2);
    let app = project.package_by_name("app").expect("app").0;
    let plan = project
        .compile_plan(app, &project.package(app).targets[0].name, &plan_options())
        .expect("plan");
    assert_eq!(plan.packages[0].name, "greet");
    assert_eq!(plan.packages[1].name, "app");
    assert_eq!(plan.packages[1].dependencies[0].alias, "greet");
    assert_eq!(plan.packages[1].dependencies[0].package, 0);
}

#[test]
fn a_dependency_cycle_is_reported() {
    let fixture = fixture(&[
        (
            "seal.toml",
            "[project]\nname = \"ws\"\nmembers = [\"crates/*\"]\n",
        ),
        (
            "crates/a/seal.toml",
            "[package]\nname = \"a\"\nversion = \"0.1.0\"\n\n\
             [dependencies]\nb = { path = \"../b\" }\n",
        ),
        ("crates/a/src/lib.sl", "func a() -> i32 { 0 }\n"),
        (
            "crates/b/seal.toml",
            "[package]\nname = \"b\"\nversion = \"0.1.0\"\n\n\
             [dependencies]\na = { path = \"../a\" }\n",
        ),
        ("crates/b/src/lib.sl", "func b() -> i32 { 0 }\n"),
    ]);
    let error = Project::load(&fixture.root).unwrap_err();
    assert_eq!(error.code(), "E8006");
}

#[test]
fn duplicate_package_names_are_reported() {
    let fixture = fixture(&[
        (
            "seal.toml",
            "[project]\nname = \"ws\"\nmembers = [\"crates/*\"]\n",
        ),
        (
            "crates/a/seal.toml",
            "[package]\nname = \"same\"\nversion = \"0.1.0\"\n",
        ),
        ("crates/a/src/lib.sl", "func a() -> i32 { 0 }\n"),
        (
            "crates/b/seal.toml",
            "[package]\nname = \"same\"\nversion = \"0.2.0\"\n",
        ),
        ("crates/b/src/lib.sl", "func b() -> i32 { 0 }\n"),
    ]);
    let error = Project::load(&fixture.root).unwrap_err();
    assert_eq!(error.code(), "E8005");
}

#[test]
fn a_source_glob_without_matches_is_reported() {
    let fixture = fixture(&[
        (
            "seal.toml",
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nsources = [\"lib/**/*.sl\"]\n",
        ),
        ("src/main.sl", "func main() -> i32 { 0 }\n"),
    ]);
    let error = Project::load(&fixture.root).unwrap_err();
    assert_eq!(error.code(), "E8004");
}

#[test]
fn an_explicit_entry_is_carried_into_the_plan() {
    let fixture = fixture(&[
        (
            "seal.toml",
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n\
             [targets.server]\nkind = \"bin\"\nentry = \"app::main\"\n",
        ),
        ("src/main.sl", "module app;\nfunc main() -> i32 { 0 }\n"),
    ]);
    let project = Project::load(&fixture.root).expect("project");
    let plan = project
        .compile_plan(0, "server", &plan_options())
        .expect("plan");
    assert_eq!(plan.entry.as_deref(), Some("app::main"));
}

#[test]
fn two_targets_cannot_claim_the_same_explicit_source() {
    let fixture = fixture(&[
        (
            "seal.toml",
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n\
             [targets.a]\nkind = \"bin\"\nentry = \"root::main\"\nsources = [\"src/main.sl\"]\n\n\
             [targets.b]\nkind = \"bin\"\nentry = \"root::main\"\nsources = [\"src/main.sl\"]\n",
        ),
        ("src/main.sl", "func main() -> i32 { 0 }\n"),
    ]);
    let error = Project::load(&fixture.root).unwrap_err();
    assert_eq!(error.code(), "E8003");
    assert!(error.message().contains("both claim"));
}

#[test]
fn a_library_plan_has_no_entry_and_uses_an_object_output() {
    let fixture = fixture(&[
        (
            "seal.toml",
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
        ),
        ("src/lib.sl", "func answer() -> i32 { 42 }\n"),
    ]);
    let project = Project::load(&fixture.root).expect("project");
    let mut options = plan_options();
    options.mode = BuildMode::Build;
    let plan = project
        .compile_plan(0, &project.package(0).targets[0].name, &options)
        .expect("plan");
    assert!(plan.entry.is_none());
    assert!(
        plan.output
            .as_ref()
            .is_some_and(|path| path.extension().is_some_and(|extension| extension == "o"))
    );
}

#[test]
fn the_lockfile_is_deterministic_and_relative() {
    let fixture = fixture(&[
        (
            "seal.toml",
            "[project]\nname = \"ws\"\nmembers = [\"crates/*\"]\n",
        ),
        (
            "crates/app/seal.toml",
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n\
             [dependencies]\ngreet = { path = \"../greet\" }\n",
        ),
        ("crates/app/src/main.sl", "func main() -> i32 { 0 }\n"),
        (
            "crates/greet/seal.toml",
            "[package]\nname = \"greet\"\nversion = \"0.1.0\"\n",
        ),
        ("crates/greet/src/lib.sl", "func answer() -> i32 { 42 }\n"),
    ]);
    let project = Project::load(&fixture.root).expect("project");
    let first = project.resolve_lockfile("x86_64-unknown-linux-gnu");
    let second = project.resolve_lockfile("x86_64-unknown-linux-gnu");
    assert_eq!(first, second);
    assert!(
        first
            .packages
            .iter()
            .all(|package| package.source.starts_with("path+"))
    );
    let text = first.render().expect("render");
    assert!(!text.contains(&fixture.root.display().to_string()));
}

#[test]
fn an_empty_member_glob_is_rejected() {
    let fixture = fixture(&[("seal.toml", "[project]\nname = \"ws\"\nmembers = [\"\"]\n")]);
    let error = Project::load(&fixture.root).unwrap_err();
    assert_eq!(error.code(), "E8005");
}

#[test]
fn the_project_root_must_contain_a_manifest() {
    let fixture = fixture(&[]);
    let error = Project::load(&fixture.root).unwrap_err();
    assert_eq!(error.code(), "E8011");
}

#[test]
fn a_member_without_a_package_is_reported() {
    let fixture = fixture(&[
        (
            "seal.toml",
            "[project]\nname = \"ws\"\nmembers = [\"crates/*\"]\n",
        ),
        ("crates/empty/seal.toml", "[project]\nname = \"other\"\n"),
    ]);
    let error = Project::load(&fixture.root).unwrap_err();
    assert_eq!(error.code(), "E8005");
}

#[test]
fn generated_files_are_added_to_the_primary_plan() {
    let fixture = fixture(&[
        (
            "seal.toml",
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n\
             [targets.gen]\nkind = \"generator\"\nentry = \"root::main\"\n\
             sources = [\"src/gen.sl\"]\ninputs = [\"data/*.txt\"]\n\
             outputs = [\"generated/table.sl\"]\n\n\
             [targets.app]\nkind = \"bin\"\nentry = \"root::main\"\n\
             sources = [\"src/main.sl\"]\ngenerated = [\"gen\"]\n",
        ),
        ("src/gen.sl", "func main() -> i32 { 0 }\n"),
        ("src/main.sl", "func main() -> i32 { value() }\n"),
        ("data/table.txt", "payload\n"),
        ("generated/table.sl", "func value() -> i32 { 0 }\n"),
    ]);
    let project = Project::load(&fixture.root).expect("project");
    let package = project.package(0);
    let generator = package.target("gen").expect("generator");
    assert_eq!(generator.generator_inputs.len(), 1);
    assert_eq!(generator.generator_outputs, vec!["generated/table.sl"]);
    let app = package.target("app").expect("app");
    assert_eq!(app.generated, vec!["gen"]);

    let generated = fixture.root.join("generated/table.sl");
    let options = PlanOptions {
        mode: BuildMode::Build,
        extra_files: vec![generated.clone()],
        ..PlanOptions::default()
    };
    let plan = project
        .compile_plan(0, "app", &options)
        .expect("the plan builds");
    assert!(
        plan.packages[0]
            .files
            .iter()
            .any(|file| file.path == generated)
    );
}

#[test]
fn output_paths_stay_under_the_project() {
    let fixture = fixture(&[
        (
            "seal.toml",
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n\
             [targets.main]\nkind = \"bin\"\nentry = \"root::main\"\noutput = \"../escape\"\n",
        ),
        ("src/main.sl", "func main() -> i32 { 0 }\n"),
    ]);
    let error = Project::load(&fixture.root).unwrap_err();
    assert_eq!(error.code(), "E8003");
}
