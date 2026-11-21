//! `seal task`: run a declarative task from `seal.toml`.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::ExitCode;

use seal_build::{TaskReport, TaskRequest, TaskRunner};
use seal_manifest::{Manifest, TaskManifest};
use seal_sandbox::Sandbox;

use crate::cli::{Cli, TaskArgs};
use crate::ui;

/// Runs one named task, its dependencies first.
pub(crate) fn task(cli: &Cli, root: &Path, args: &TaskArgs) -> ExitCode {
    let project = match crate::cli::load(cli, root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let manifest = match seal_manifest::load(&project.root.join("seal.toml")) {
        Ok(loaded) => loaded.manifest,
        Err(error) => return ui::error("E8005", &error.to_string()),
    };
    let Some(task) = manifest.tasks.get(&args.name) else {
        return missing(&manifest, &args.name);
    };
    if let Some(description) = &task.description {
        ui::status(cli, description);
    }
    let runner = TaskRunner::new(&project.root, Sandbox::detect());
    let mut stack = Vec::new();
    let mut completed = BTreeSet::new();
    match run_named(
        cli,
        &manifest,
        &runner,
        &args.name,
        &args.args,
        !args.no_fail_fast,
        &mut stack,
        &mut completed,
    ) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(failed) => ui::error(
            "E8046",
            &format!("the task `{}` had {failed} failing step(s)", args.name),
        ),
        Err(code) => code,
    }
}

fn missing(manifest: &Manifest, name: &str) -> ExitCode {
    let available: Vec<&str> = manifest.tasks.keys().map(String::as_str).collect();
    let message = if available.is_empty() {
        format!("the project has no task `{name}`; declare one under `[tasks]`")
    } else {
        format!(
            "the project has no task `{name}`; available: {}",
            available.join(", ")
        )
    };
    ui::error("E8046", &message)
}

#[allow(clippy::too_many_arguments)]
fn run_named(
    cli: &Cli,
    manifest: &Manifest,
    runner: &TaskRunner,
    name: &str,
    args: &[String],
    fail_fast: bool,
    stack: &mut Vec<String>,
    completed: &mut BTreeSet<String>,
) -> Result<usize, ExitCode> {
    if completed.contains(name) {
        return Ok(0);
    }
    if stack.iter().any(|item| item == name) {
        let mut cycle = stack.clone();
        cycle.push(name.to_owned());
        return Err(ui::error(
            "E8046",
            &format!("the task graph contains a cycle: {}", cycle.join(" -> ")),
        ));
    }
    let Some(task) = manifest.tasks.get(name) else {
        return Err(ui::error(
            "E8046",
            &format!("the task graph names the unknown task `{name}`"),
        ));
    };
    stack.push(name.to_owned());
    let mut failed = 0;
    for dependency in &task.depends_on {
        let dependency_failed = run_named(
            cli, manifest, runner, dependency, args, fail_fast, stack, completed,
        )?;
        failed += dependency_failed;
        if dependency_failed > 0 && fail_fast {
            stack.pop();
            return Ok(failed);
        }
    }
    let report = match runner.run(
        &TaskRequest {
            name: name.to_owned(),
            args: args.to_vec(),
        },
        task,
        fail_fast,
    ) {
        Ok(report) => report,
        Err(error) => {
            stack.pop();
            return Err(ui::error(error.code(), error.message()));
        }
    };
    report_outcomes(cli, &report, task);
    failed += report.failed;
    stack.pop();
    // A task that already ran is not run again when a diamond dependency
    // reaches it; a failure is still reported through the first branch.
    completed.insert(name.to_owned());
    Ok(failed)
}

fn report_outcomes(cli: &Cli, report: &TaskReport, task: &TaskManifest) {
    let description = task
        .description
        .as_deref()
        .map(|description| format!("{description}: "))
        .unwrap_or_default();
    for outcome in &report.outcomes {
        if outcome.success {
            ui::status(
                cli,
                &format!(
                    "{description}{} step {}: {} ok ({} ms)",
                    report.name, outcome.index, outcome.program, outcome.millis
                ),
            );
            if cli.verbose && !outcome.stdout.is_empty() {
                print!("{}", outcome.stdout);
            }
        } else {
            eprintln!(
                "{description}{} step {}: {} failed ({})",
                report.name,
                outcome.index,
                outcome.program,
                outcome
                    .status
                    .map_or_else(|| "signal".to_owned(), |code| code.to_string())
            );
            if !outcome.stderr.is_empty() {
                eprint!("{}", outcome.stderr);
            }
        }
    }
}
