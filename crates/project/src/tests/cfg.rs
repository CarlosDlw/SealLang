//! Target and dependency configuration conditions.

use super::{fixture, load, options};

fn project_files(target_when: &str, dependency_when: &str) -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "seal.toml",
            "[project]\nname = \"cfg\"\nmembers = [\"crates/*\"]\ndefault-members = [\"crates/app\"]\n",
        ),
        (
            "crates/common/seal.toml",
            "[package]\nname = \"common\"\nversion = \"0.1.0\"\n",
        ),
        (
            "crates/common/src/lib.sl",
            "public func value() -> i32 { 1 }\n",
        ),
        (
            "crates/app/seal.toml",
            Box::leak(
                format!(
                    "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\ncommon = {{ path = \"../common\"{dependency_when} }}\n\n[targets.app]\nkind = \"bin\"\nentry = \"root::main\"\n{target_when}\n"
                )
                .into_boxed_str(),
            ) as &'static str,
        ),
        ("crates/app/src/main.sl", "func main() -> i32 { 0 }\n"),
    ]
}

#[test]
fn an_inactive_target_is_not_compiled() {
    let fixture = fixture(&project_files("when = { os = \"linux\" }", ""));
    let project = load(&fixture);
    let package = project.default_package().expect("a default package");
    let mut plan_options = options(&[]);
    plan_options.target = "x86_64-pc-windows-msvc".to_owned();
    let error = project
        .compile_plan(package, "app", &plan_options)
        .expect_err("the target is inactive");
    assert_eq!(error.code(), "E8003");

    plan_options.target = "x86_64-unknown-linux-gnu".to_owned();
    project
        .compile_plan(package, "app", &plan_options)
        .expect("the target is active");
}

#[test]
fn a_command_line_cfg_fact_is_recorded_in_the_plan() {
    let fixture = fixture(&[
        (
            "seal.toml",
            "[project]\nname = \"cfg\"\n\n[package]\nname = \"cfg\"\nversion = \"0.1.0\"\n",
        ),
        (
            "src/main.sl",
            "cfg(test) public func helper() -> i32 { 1 }\nfunc main() -> i32 { 0 }\n",
        ),
    ]);
    let project = load(&fixture);
    let package = project.default_package().expect("a default package");
    let mut plan_options = options(&[]);
    plan_options.cfgs.push("test".to_owned());
    let plan = project
        .compile_plan(package, "cfg", &plan_options)
        .expect("the plan is valid");
    assert!(plan.packages[0].cfg.test);
}

#[test]
fn an_inactive_dependency_is_not_visited() {
    let fixture = fixture(&project_files("", ", when = { os = \"linux\" }"));
    let project = load(&fixture);
    let package = project.default_package().expect("a default package");
    let linux = project
        .compile_plan(package, "app", &options(&[]))
        .expect("linux is active");
    assert!(
        linux
            .packages
            .iter()
            .any(|package| package.name == "common")
    );

    let mut windows = options(&[]);
    windows.target = "x86_64-pc-windows-msvc".to_owned();
    let plan = project
        .compile_plan(package, "app", &windows)
        .expect("windows is valid");
    assert!(!plan.packages.iter().any(|package| package.name == "common"));
}
