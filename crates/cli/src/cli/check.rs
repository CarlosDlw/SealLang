//! `seal check`: analyze targets without writing artifacts.

use std::process::ExitCode;

use seal_plan::BuildMode;

use crate::cli::build::{Mode, select_jobs};
use crate::cli::{CheckArgs, Cli};
use crate::ui;

pub(crate) fn check(cli: &Cli, root: &std::path::Path, args: &CheckArgs) -> ExitCode {
    let project = match crate::cli::load_for_selection(cli, root, &args.selection) {
        Ok(project) => project,
        Err(code) => return code,
    };
    for warning in &project.warnings {
        if !cli.quiet {
            ui::warning(&warning.message);
        }
    }
    let jobs = match select_jobs(&project, &args.selection, Mode::Check) {
        Ok(jobs) => jobs,
        Err(code) => return code,
    };
    let target_triple = args
        .selection
        .target_triple
        .clone()
        .unwrap_or_else(|| seal_target::host_triple().as_str().to_owned());
    let toolchain = match crate::cli::build::toolchain_version(&project, &target_triple) {
        Ok(toolchain) => toolchain,
        Err(code) => return code,
    };
    if let Err(code) =
        crate::cli::build::prepare_lockfile(cli, &project, &target_triple, &args.selection)
    {
        return code;
    }
    for job in jobs {
        let target = project
            .package(job.package)
            .target(&job.target)
            .expect("selection returns existing targets");
        let mut options =
            crate::cli::build::plan_options(&args.selection, BuildMode::Check, target);
        options.toolchain = toolchain.clone();
        let plan = match project.compile_plan(job.package, &job.target, &options) {
            Ok(plan) => plan,
            Err(error) => return ui::error(error.code(), error.message()),
        };
        ui::status(
            cli,
            &format!(
                "Checking {} {}",
                project.package(job.package).name,
                job.target
            ),
        );
        if let Err(code) = crate::cli::build::run_plan(cli, &plan, Mode::Check, "obj", &[]) {
            return code;
        }
    }
    ExitCode::SUCCESS
}
