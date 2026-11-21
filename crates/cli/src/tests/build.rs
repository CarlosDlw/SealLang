//! Target selection and plan-option tests for build-like commands.

use std::collections::BTreeSet;

use seal_plan::BuildMode;

use crate::cli::build::{Mode, plan_options, select_jobs};

use super::{load, monorepo, sample_project, selection};

fn job_names(jobs: &[crate::cli::build::Job]) -> BTreeSet<&str> {
    jobs.iter().map(|job| job.target.as_str()).collect()
}

#[test]
fn build_defaults_select_libraries_and_binaries_only() {
    let fixture = sample_project("build-defaults");
    let project = load(&fixture);
    let jobs = select_jobs(&project, &selection(), Mode::Build).expect("build selection");
    assert_eq!(
        job_names(&jobs),
        BTreeSet::from(["main", "share"]),
        "tests, benches, examples, and generators are opt-in"
    );
}

#[test]
fn all_targets_includes_tests_benches_examples_and_generators() {
    let fixture = sample_project("build-all-targets");
    let project = load(&fixture);
    let mut selection = selection();
    selection.all_targets = true;
    let jobs = select_jobs(&project, &selection, Mode::Build).expect("all-targets selection");
    assert_eq!(
        job_names(&jobs),
        BTreeSet::from(["checks", "codegen", "demo", "main", "measure", "share"]),
        "--all-targets covers every declared target kind"
    );
}

#[test]
fn check_defaults_match_build_defaults() {
    let fixture = sample_project("check-defaults");
    let project = load(&fixture);
    let jobs = select_jobs(&project, &selection(), Mode::Check).expect("check selection");
    assert_eq!(job_names(&jobs), BTreeSet::from(["main", "share"]));
}

#[test]
fn an_explicit_target_name_selects_only_that_target() {
    let fixture = sample_project("build-explicit");
    let project = load(&fixture);
    let mut selection = selection();
    selection.targets.push("demo".to_owned());
    let jobs = select_jobs(&project, &selection, Mode::Build).expect("explicit selection");
    assert_eq!(job_names(&jobs), BTreeSet::from(["demo"]));
}

#[test]
fn a_command_line_profile_overrides_the_target_profile() {
    let fixture = sample_project("build-profile");
    let project = load(&fixture);
    let package = project.package(0);
    let main = package.target("main").expect("main target");
    let share = package.target("share").expect("share target");

    let mut selection = selection();
    selection.profile = Some("custom".to_owned());
    assert_eq!(
        plan_options(&selection, BuildMode::Build, main).profile,
        "custom"
    );
    assert_eq!(
        plan_options(&selection, BuildMode::Build, share).profile,
        "custom"
    );

    selection.release = true;
    selection.profile = None;
    assert_eq!(
        plan_options(&selection, BuildMode::Build, share).profile,
        "release"
    );
}

#[test]
fn default_members_select_every_declared_member() {
    let fixture = monorepo("build-default-members");
    let project = load(&fixture);
    let jobs = select_jobs(&project, &selection(), Mode::Build).expect("default members");
    assert_eq!(job_names(&jobs), BTreeSet::from(["alpha", "beta"]));
}

#[test]
fn workspace_and_exclude_select_the_right_packages() {
    let fixture = monorepo("build-exclude");
    let project = load(&fixture);
    let mut selection = selection();
    selection.workspace = true;
    selection.exclude = vec!["beta".to_owned(), "app".to_owned()];
    let jobs = select_jobs(&project, &selection, Mode::Build).expect("workspace selection");
    assert_eq!(job_names(&jobs), BTreeSet::from(["alpha"]));

    selection.exclude = vec!["missing".to_owned()];
    assert!(select_jobs(&project, &selection, Mode::Build).is_err());
}

#[test]
fn a_selection_that_excludes_everything_is_an_error() {
    let fixture = monorepo("build-exclude-all");
    let project = load(&fixture);
    let mut selection = selection();
    selection.workspace = true;
    selection.exclude = vec!["alpha".to_owned(), "beta".to_owned(), "app".to_owned()];
    assert!(select_jobs(&project, &selection, Mode::Build).is_err());
}

#[test]
fn a_target_profile_selects_optimization_values() {
    let fixture = sample_project("build-profile-values");
    let project = load(&fixture);
    let target = project.package(0).target("main").expect("main target");
    let options = plan_options(&selection(), BuildMode::Build, target);
    let plan = project
        .compile_plan(0, "main", &options)
        .expect("the plan resolves");
    assert_eq!(plan.profile.opt_level, 3);
    assert!(!plan.profile.debug);
}
