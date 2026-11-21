//! `seal search`: query the signed registry catalog.

use std::path::Path;
use std::process::ExitCode;

use crate::cli::{Cli, SearchArgs};
use crate::ui;

pub(crate) fn run(cli: &Cli, root: &Path, args: &SearchArgs) -> ExitCode {
    let session = match crate::cli::registry::open(cli, root) {
        Ok(session) => session,
        Err(code) => return code,
    };
    let entries = match session.search(&args.query) {
        Ok(entries) => entries,
        Err(error) => return ui::error(error.code(), error.message()),
    };
    let entries: Vec<_> = entries.into_iter().take(args.limit).collect();
    if args.json {
        match serde_json::to_string_pretty(&entries) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                return ui::error(
                    "E8032",
                    &format!("cannot serialize search results: {error}"),
                );
            }
        }
        return ExitCode::SUCCESS;
    }
    if entries.is_empty() {
        println!("no packages match `{}`", args.query);
        return ExitCode::SUCCESS;
    }
    for entry in entries {
        let version = entry.latest.as_deref().unwrap_or("-");
        let description = entry.description.unwrap_or_default();
        if description.is_empty() {
            println!("{} {}", entry.name, version);
        } else {
            println!("{} {} - {}", entry.name, version, description);
        }
    }
    ExitCode::SUCCESS
}
