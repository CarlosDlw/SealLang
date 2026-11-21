//! `seal logout`: remove a stored registry token.

use std::path::Path;
use std::process::ExitCode;

use crate::cli::{Cli, LogoutArgs};
use crate::ui;

pub(crate) fn run(cli: &Cli, root: &Path, args: &LogoutArgs) -> ExitCode {
    let project = seal_project::Project::load(root).ok();
    let manifest = project
        .as_ref()
        .and_then(|project| project.registry.clone());
    let name = args
        .name
        .clone()
        .unwrap_or_else(|| super::login::registry_name(manifest.as_ref()));
    let project_root = project
        .as_ref()
        .map(|project| project.root.clone())
        .unwrap_or_else(|| root.to_path_buf());
    match seal_registry::auth::remove_token(
        &project_root,
        manifest
            .as_ref()
            .and_then(|manifest| manifest.credentials.as_deref()),
        &name,
    ) {
        Ok(true) => {
            ui::status(cli, &format!("removed the token for `{name}`"));
            ExitCode::SUCCESS
        }
        Ok(false) => {
            ui::status(cli, &format!("no token was stored for `{name}`"));
            ExitCode::SUCCESS
        }
        Err(error) => ui::error(error.code(), error.message()),
    }
}
