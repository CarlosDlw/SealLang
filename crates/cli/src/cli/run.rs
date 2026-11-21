//! `seal run`: build one executable and run it.

use std::process::{Command, ExitCode};

use seal_plan::BuildMode;

use crate::cli::build::{Mode, plan_options, prepare_lockfile, select_jobs};
use crate::cli::{Cli, RunArgs};
use crate::ui;

pub(crate) fn run(cli: &Cli, root: &std::path::Path, args: &RunArgs) -> ExitCode {
    let project = match crate::cli::load(root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let jobs = if let Some(name) = &args.bin {
        if !args.selection.targets.is_empty() {
            return ui::error(
                "E8103",
                "`--bin` and an explicit target name cannot be combined",
            );
        }
        let package = if let Some(package) = &args.selection.package {
            match project.package_by_name(package) {
                Some((index, _)) => index,
                None => {
                    return ui::error(
                        "E8005",
                        &format!("the project has no package named `{package}`"),
                    );
                }
            }
        } else {
            match project.default_package() {
                Ok(package) => package,
                Err(error) => return ui::error(error.code(), error.message()),
            }
        };
        if project.package(package).target(name).is_none() {
            return ui::error(
                "E8003",
                &format!(
                    "package `{}` has no target `{name}`",
                    project.package(package).name
                ),
            );
        }
        vec![crate::cli::build::Job {
            package,
            target: name.clone(),
        }]
    } else {
        match select_jobs(&project, &args.selection, Mode::Run) {
            Ok(jobs) => jobs,
            Err(code) => return code,
        }
    };
    if jobs.len() != 1 {
        return ui::error(
            "E8103",
            "`seal run` needs exactly one executable target; select one with `--bin`",
        );
    }
    let job = &jobs[0];
    let target = project
        .package(job.package)
        .target(&job.target)
        .expect("selection returns existing targets");
    if !target.is_executable() {
        return ui::error(
            "E8103",
            &format!(
                "the target `{}` does not produce an executable",
                target.name
            ),
        );
    }
    let triple = args
        .selection
        .target_triple
        .clone()
        .unwrap_or_else(|| seal_target::host_triple().as_str().to_owned());
    let toolchain = match crate::cli::build::toolchain_version(&project, &triple) {
        Ok(toolchain) => toolchain,
        Err(code) => return code,
    };
    if let Err(code) = prepare_lockfile(cli, &project, &triple, &args.selection) {
        return code;
    }
    let mut options = plan_options(&args.selection, BuildMode::Build, target);
    options.toolchain = toolchain;
    let plan = match project.compile_plan(job.package, &job.target, &options) {
        Ok(plan) => plan,
        Err(error) => return ui::error(error.code(), error.message()),
    };
    let output = match crate::cli::build::run_plan(cli, &plan, Mode::Run, "exe", &[]) {
        Ok(output) => output,
        Err(code) => return code,
    };
    ui::verbose(cli, &format!("running: {}", output.display()));
    match Command::new(&output).args(&args.program_args).status() {
        Ok(status) => ui::exit_code(status),
        Err(error) => ui::error(
            "E8011",
            &format!("cannot execute `{}`: {error}", output.display()),
        ),
    }
}
