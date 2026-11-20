//! Selection tests for `seal run`.

use std::collections::BTreeSet;

use crate::cli::build::{Mode, select_jobs};

use super::{load, sample_project, selection};

fn job_names(jobs: &[crate::cli::build::Job]) -> BTreeSet<&str> {
    jobs.iter().map(|job| job.target.as_str()).collect()
}

#[test]
fn run_selection_contains_every_executable_target() {
    let fixture = sample_project("run-executables");
    let project = load(&fixture);
    let jobs = select_jobs(&project, &selection(), Mode::Run).expect("run selection");
    assert_eq!(
        job_names(&jobs),
        BTreeSet::from(["checks", "codegen", "demo", "main", "measure"]),
        "libraries are excluded from run selection"
    );
}

#[test]
fn an_explicit_binary_narrows_the_run_selection_to_one_job() {
    let fixture = sample_project("run-explicit");
    let project = load(&fixture);
    let mut selection = selection();
    selection.targets.push("main".to_owned());
    let jobs = select_jobs(&project, &selection, Mode::Run).expect("explicit run selection");
    assert_eq!(job_names(&jobs), BTreeSet::from(["main"]));
    assert_eq!(jobs.len(), 1);
}
