//! Profile inheritance and command-line precedence.

use super::{fixture, load, options};

#[test]
fn a_root_profile_inherits_and_overrides_values() {
    let fixture = fixture(&[
        (
            "seal.toml",
            "[project]\nname = \"profiles\"\n\n[package]\nname = \"profiles\"\nversion = \"0.1.0\"\n\n[profiles.fast]\ninherits = \"release\"\nopt-level = 3\ndebug = true\n",
        ),
        ("src/main.sl", "func main() -> i32 { 0 }\n"),
    ]);
    let project = load(&fixture);
    let mut plan_options = options(&[]);
    plan_options.profile = "fast".to_owned();
    plan_options.profile_explicit = true;
    let plan = project
        .compile_plan(
            project.default_package().expect("a default package"),
            "profiles",
            &plan_options,
        )
        .expect("the plan resolves");
    assert_eq!(plan.profile.opt_level, 3);
    assert!(plan.profile.debug);
}

#[test]
fn an_unknown_target_profile_is_reported_by_the_plan() {
    let fixture = fixture(&[
        (
            "seal.toml",
            "[project]\nname = \"profiles\"\n\n[package]\nname = \"profiles\"\nversion = \"0.1.0\"\n\n[targets.main]\nkind = \"bin\"\nentry = \"root::main\"\nprofile = \"missing\"\n",
        ),
        ("src/main.sl", "func main() -> i32 { 0 }\n"),
    ]);
    let project = load(&fixture);
    let error = project
        .compile_plan(
            project.default_package().expect("a default package"),
            "main",
            &options(&[]),
        )
        .expect_err("the target profile is unknown");
    assert_eq!(error.code(), "E8009");
}

#[test]
fn an_unknown_profile_is_reported() {
    let fixture = fixture(&[
        (
            "seal.toml",
            "[project]\nname = \"profiles\"\n\n[package]\nname = \"profiles\"\nversion = \"0.1.0\"\n",
        ),
        ("src/main.sl", "func main() -> i32 { 0 }\n"),
    ]);
    let project = load(&fixture);
    let mut plan_options = options(&[]);
    plan_options.profile = "missing".to_owned();
    plan_options.profile_explicit = true;
    let error = project
        .compile_plan(
            project.default_package().expect("a default package"),
            "profiles",
            &plan_options,
        )
        .expect_err("the profile is unknown");
    assert_eq!(error.code(), "E8009");
}
