//! `seal lint`: run the compiler's stable lint policy over a project.
//!
//! Compiler warnings are the source of truth. This command adds names for the
//! lints that can be denied without a second analysis pass and maps them to the
//! driver's `--deny-warnings` policy. The `unused_import` lint is currently the
//! only individual lint the compiler emits, so denying it and denying the
//! `warnings` group have the same effect; the distinct names keep scripts
//! stable when more lints exist.

use std::process::ExitCode;

use crate::cli::check::check_jobs_with;
use crate::cli::{Cli, LintArgs};
use crate::ui;

/// The lints this command understands.
const LINTS: [&str; 2] = ["unused_import", "warnings"];

pub(crate) fn lint(cli: &Cli, root: &std::path::Path, args: &LintArgs) -> ExitCode {
    if let Err(code) = validate(&args.deny, &args.allow) {
        return code;
    }
    let project = match crate::cli::load_for_selection(cli, root, &args.selection) {
        Ok(project) => project,
        Err(code) => return code,
    };
    for warning in &project.warnings {
        if !cli.quiet {
            ui::warning(&warning.message);
        }
    }
    let jobs = match crate::cli::build::select_jobs(
        &project,
        &args.selection,
        crate::cli::build::Mode::Check,
    ) {
        Ok(jobs) => jobs,
        Err(code) => return code,
    };
    let deny = !args.deny.is_empty();
    let extra: Vec<String> = deny
        .then(|| "--deny-warnings".to_owned())
        .into_iter()
        .collect();
    let code = check_jobs_with(cli, &project, &jobs, &args.selection, &extra, !args.json);
    let denied = code != ExitCode::SUCCESS;
    if args.json {
        let value = serde_json::json!({
            "status": if denied { "denied" } else { "ok" },
            "denied_lints": args.deny,
            "allowed_lints": args.allow,
        });
        match serde_json::to_string_pretty(&value) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                return ui::error(
                    "E8108",
                    &format!("cannot serialize the lint report: {error}"),
                );
            }
        }
    }
    code
}

fn validate(deny: &[String], allow: &[String]) -> Result<(), ExitCode> {
    for lint in deny.iter().chain(allow) {
        if !LINTS.contains(&lint.as_str()) {
            return Err(ui::error(
                "E8112",
                &format!(
                    "unknown lint `{lint}`; the stable lints are {}",
                    LINTS.join(", ")
                ),
            ));
        }
    }
    for lint in deny {
        if allow.contains(lint) {
            return Err(ui::error(
                "E8112",
                &format!("the lint `{lint}` is both denied and allowed"),
            ));
        }
    }
    Ok(())
}
