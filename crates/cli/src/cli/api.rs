//! `seal api`: print the public API snapshot and its digest.

use std::path::Path;
use std::process::ExitCode;

use crate::cli::{ApiArgs, Cli};
use crate::ui;

pub(crate) fn run(cli: &Cli, root: &Path, args: &ApiArgs) -> ExitCode {
    let project = match crate::cli::load(cli, root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let snapshot = match crate::cli::snapshot::generate(cli, &project, args.package.as_deref()) {
        Ok(snapshot) => snapshot,
        Err(code) => return code,
    };
    if args.json {
        match serde_json::to_string_pretty(&snapshot) {
            Ok(text) => {
                println!("{text}");
                ExitCode::SUCCESS
            }
            Err(error) => ui::error("E8090", &format!("cannot serialize the snapshot: {error}")),
        }
    } else {
        match snapshot.hash() {
            Ok(hash) => {
                println!(
                    "{} {}: {} public item(s), {} module(s), {}",
                    snapshot.package,
                    snapshot.package_version,
                    snapshot.items.len(),
                    snapshot.modules.len(),
                    hash
                );
                ExitCode::SUCCESS
            }
            Err(error) => ui::error(error.code(), error.message()),
        }
    }
}
