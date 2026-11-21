//! `seal clean`: remove build outputs and the local cache.

use std::path::Path;
use std::process::ExitCode;

use crate::cli::Cli;
use crate::ui;

/// Removes `seal/cache` and `seal/target`.
pub(crate) fn clean(cli: &Cli, root: &Path) -> ExitCode {
    let project = match crate::cli::load(cli, root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    match seal_build::clean(&project.root) {
        Ok(removed) => {
            if removed.is_empty() {
                ui::status(cli, "nothing to clean");
            }
            for path in removed {
                ui::status(cli, &format!("removed {}", path.display()));
            }
            ExitCode::SUCCESS
        }
        Err(error) => ui::error(error.code(), error.message()),
    }
}
