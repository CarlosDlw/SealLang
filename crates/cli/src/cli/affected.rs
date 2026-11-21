//! `seal affected`: map a VCS diff to packages and targets.

use std::path::Path;
use std::process::ExitCode;

use seal_project::AffectedReport;

use crate::cli::build::Job;
use crate::cli::{AffectedArgs, Cli, Selection};
use crate::ui;

pub(crate) fn affected(cli: &Cli, root: &Path, args: &AffectedArgs) -> ExitCode {
    let project = match crate::cli::load(cli, root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let report = match seal_project::affected(&project, &args.since) {
        Ok(report) => report,
        Err(error) => return ui::error(error.code(), error.message()),
    };
    if args.json {
        match serde_json::to_string_pretty(&report) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                return ui::error(
                    "E8012",
                    &format!("cannot serialize the affected report: {error}"),
                );
            }
        }
    } else {
        print_report(&report);
    }
    if !args.run {
        return ExitCode::SUCCESS;
    }
    if report.targets.is_empty() {
        ui::status(cli, "No affected targets");
        return ExitCode::SUCCESS;
    }
    let jobs: Vec<Job> = report
        .targets
        .iter()
        .map(|target| Job {
            package: target.package,
            target: target.target.clone(),
        })
        .collect();
    let mut selection = Selection::default_for_command();
    selection.target_triple = args.target_triple.clone();
    selection.features = args.features.clone();
    selection.all_features = args.all_features;
    selection.no_default_features = args.no_default_features;
    crate::cli::check::check_jobs(cli, &project, &jobs, &selection)
}

fn print_report(report: &AffectedReport) {
    println!(
        "{} changed path(s) since {}, {} package(s), {} target(s)",
        report.changed.len(),
        report.since,
        report.packages.len(),
        report.targets.len()
    );
    for path in &report.changed {
        println!("  changed {path}");
    }
    for target in &report.targets {
        println!(
            "  affected {}:{} ({})",
            target.package_name, target.target, target.kind
        );
    }
}
