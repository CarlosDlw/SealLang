//! Shared registry-session setup for query commands.

use std::path::Path;
use std::process::ExitCode;

use seal_project::Project;
use seal_registry::RegistrySession;

use crate::cli::Cli;
use crate::ui;

/// Opens the project's configured registry.
pub(crate) fn session(cli: &Cli, project: &Project) -> Result<RegistrySession, ExitCode> {
    match RegistrySession::open(&project.root, project.registry.as_ref(), cli.offline) {
        Ok(Some(session)) => Ok(session),
        Ok(None) => Err(ui::error(
            "E8020",
            "the project has no registry; configure `[registry]` or `SEAL_REGISTRY_URL`",
        )),
        Err(error) => Err(ui::error(error.code(), error.message())),
    }
}

/// Loads the project and opens its registry.
pub(crate) fn open(cli: &Cli, root: &Path) -> Result<RegistrySession, ExitCode> {
    let project = crate::cli::load(cli, root)?;
    session(cli, &project)
}
