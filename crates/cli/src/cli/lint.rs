//! `seal lint`: run the compiler's stable lint policy over a project.
//!
//! Compiler warnings are the source of truth. This command maps individual lint
//! names to the driver's `--allow-lint` and `--deny-lint` flags, and maps the
//! `warnings` group to the existing `--deny-warnings` policy. The names come from
//! `seal-diagnostics`, so `seal lint` and the compiler cannot drift apart.

use std::process::ExitCode;

use seal_diagnostics::{ALL_WARNINGS, DEPRECATED, UNUSED_IMPORT};

use crate::cli::check::check_jobs_with;
use crate::cli::{Cli, LintArgs};
use crate::ui;

/// The lints this command understands.
const LINTS: [&str; 3] = [UNUSED_IMPORT, DEPRECATED, ALL_WARNINGS];

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
    let mut extra = Vec::new();
    if args.deny.iter().any(|lint| lint == ALL_WARNINGS) {
        extra.push("--deny-warnings".to_owned());
    }
    for lint in &args.deny {
        if lint != ALL_WARNINGS {
            extra.push("--deny-lint".to_owned());
            extra.push(lint.clone());
        }
    }
    for lint in &args.allow {
        if lint != ALL_WARNINGS {
            extra.push("--allow-lint".to_owned());
            extra.push(lint.clone());
        }
    }
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
    if allow.iter().any(|lint| lint == ALL_WARNINGS) {
        return Err(ui::error(
            "E8112",
            "the `warnings` group cannot be allowed; allow individual lints instead",
        ));
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
