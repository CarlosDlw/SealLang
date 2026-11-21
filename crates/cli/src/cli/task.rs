//! `seal task`: run a declarative task from `seal.toml`.

use std::path::Path;
use std::process::ExitCode;

use seal_build::{TaskRequest, TaskRunner};
use seal_sandbox::Sandbox;

use crate::cli::{Cli, TaskArgs};
use crate::ui;

/// Runs one named task.
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
        let available: Vec<&str> = manifest.tasks.keys().map(String::as_str).collect();
        let message = if available.is_empty() {
            format!(
                "the project has no task `{}`; declare one under `[tasks]`",
                args.name
            )
        } else {
            format!(
                "the project has no task `{}`; available: {}",
                args.name,
                available.join(", ")
            )
        };
        return ui::error("E8046", &message);
    };
    if let Some(description) = &task.description {
        ui::status(cli, description);
    }
    let runner = TaskRunner::new(&project.root, Sandbox::detect());
    let report = match runner.run(
        &TaskRequest {
            name: args.name.clone(),
        },
        task,
        !args.no_fail_fast,
    ) {
        Ok(report) => report,
        Err(error) => return ui::error(error.code(), error.message()),
    };
    for outcome in &report.outcomes {
        if outcome.success {
            ui::status(
                cli,
                &format!(
                    "step {}: {} ok ({} ms)",
                    outcome.index, outcome.program, outcome.millis
                ),
            );
            if cli.verbose && !outcome.stdout.is_empty() {
                print!("{}", outcome.stdout);
            }
        } else {
            eprintln!(
                "step {}: {} failed ({})",
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
    if report.success() {
        ExitCode::SUCCESS
    } else {
        ui::error(
            "E8046",
            &format!(
                "the task `{}` had {} failing step(s)",
                report.name, report.failed
            ),
        )
    }
}
