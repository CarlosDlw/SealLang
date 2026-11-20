//! `seal tree`: the resolved package-instance graph as text.

use std::process::ExitCode;

use seal_plan::{BuildMode, CompilePlan};
use seal_project::{PlanOptions, Project};
use seal_target::host_triple;

use crate::cli::{Cli, QueryArgs};
use crate::ui;

pub(crate) fn tree(cli: &Cli, root: &std::path::Path, args: &QueryArgs) -> ExitCode {
    let project = match crate::cli::load(root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let plan = match compile(&project, args) {
        Ok(plan) => plan,
        Err(error) => return ui::error(error.code(), error.message()),
    };
    let primary = plan.primary;
    let mut lines = Vec::new();
    print_package(&plan, primary, "", true, &mut lines);
    for line in lines {
        println!("{line}");
    }
    let _ = cli;
    ExitCode::SUCCESS
}

pub(crate) fn compile(
    project: &Project,
    args: &QueryArgs,
) -> seal_project::ProjectResult<CompilePlan> {
    compile_inner(project, args, true)
}

pub(crate) fn compile_graph(
    project: &Project,
    args: &QueryArgs,
) -> seal_project::ProjectResult<CompilePlan> {
    compile_inner(project, args, false)
}

fn compile_inner(
    project: &Project,
    args: &QueryArgs,
    select_package: bool,
) -> seal_project::ProjectResult<CompilePlan> {
    let package = if let Some(name) = args.package.as_ref().filter(|_| select_package) {
        project
            .package_by_name(name)
            .map(|(index, _)| index)
            .ok_or_else(|| {
                seal_project::ProjectError::manifest(format!(
                    "the project has no package named `{name}`"
                ))
            })?
    } else {
        project.default_package()?
    };
    let triple = args
        .target_triple
        .clone()
        .unwrap_or_else(|| host_triple().as_str().to_owned());
    let target = project
        .package(package)
        .targets
        .iter()
        .find(|target| target.is_active_for(&triple))
        .ok_or_else(|| {
            seal_project::ProjectError::target(
                "the package has no target active for the selected triple",
            )
        })?;
    let options = PlanOptions {
        target: triple,
        features: args.features.clone(),
        no_default_features: args.no_default_features,
        all_features: args.all_features,
        cfgs: args.cfgs.clone(),
        mode: BuildMode::Check,
        ..PlanOptions::default()
    };
    project.compile_plan(package, &target.name, &options)
}

fn print_package(
    plan: &CompilePlan,
    index: usize,
    prefix: &str,
    last: bool,
    lines: &mut Vec<String>,
) {
    let Some(package) = plan.packages.get(index) else {
        return;
    };
    let marker = if last { "└── " } else { "├── " };
    let features = if package.cfg.features.is_empty() {
        String::new()
    } else {
        format!(" [{}]", package.cfg.features.join(","))
    };
    lines.push(format!(
        "{prefix}{marker}{} {}{features}",
        package.name, package.version
    ));
    let continuation = format!("{prefix}{}", if last { "    " } else { "│   " });
    let dependencies = &package.dependencies;
    for (position, dependency) in dependencies.iter().enumerate() {
        let child_last = position + 1 == dependencies.len();
        print_package(plan, dependency.package, &continuation, child_last, lines);
    }
}
