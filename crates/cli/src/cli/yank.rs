//! `seal yank`: mark a published version as yanked or restore it.

use std::path::Path;
use std::process::ExitCode;

use seal_registry::publish::{YankRequest, yank as send};

use crate::cli::{Cli, YankArgs};
use crate::ui;

pub(crate) fn run(cli: &Cli, root: &Path, args: &YankArgs) -> ExitCode {
    let (name, version) = match args.spec.rsplit_once('@') {
        Some((name, version)) if !name.is_empty() && !version.is_empty() => {
            (name.to_owned(), version.to_owned())
        }
        _ => {
            return ui::error(
                "E8093",
                "write the version as `name@version`, for example `foo@1.2.3`",
            );
        }
    };
    let project = match crate::cli::load(cli, root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let session = match seal_registry::RegistrySession::open(
        &project.root,
        project.registry.as_ref(),
        cli.offline,
    ) {
        Ok(Some(session)) => session,
        Ok(None) => {
            return ui::error(
                "E8020",
                "the project has no registry; configure `[registry]` or `SEAL_REGISTRY_URL`",
            );
        }
        Err(error) => return ui::error(error.code(), error.message()),
    };
    let request = YankRequest {
        name: name.clone(),
        version: version.clone(),
        yanked: !args.undo,
    };
    match send(&session, &request) {
        Ok(response) => {
            let action = if response.yanked {
                "yanked"
            } else {
                "restored"
            };
            ui::status(cli, &format!("{action} {name} {version}"));
            ui::detail(
                cli,
                &format!(
                    "  log entry {} in tree {}",
                    response.log.index, response.log.tree_size
                ),
            );
            ExitCode::SUCCESS
        }
        Err(error) => ui::error(error.code(), error.message()),
    }
}
