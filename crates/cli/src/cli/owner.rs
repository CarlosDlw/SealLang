//! `seal owner`: claim namespaces and manage their owners.

use std::path::Path;
use std::process::ExitCode;

use seal_registry::RegistrySession;
use seal_registry::publish::{NamespaceProof, OwnerRequest, OwnerResponse, owners as send_owners};

use crate::cli::{Cli, OwnerArgs};
use crate::ui;

pub(crate) fn run(cli: &Cli, root: &Path, args: &OwnerArgs) -> ExitCode {
    let project = match crate::cli::load(cli, root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let session = match RegistrySession::open(&project.root, project.registry.as_ref(), cli.offline)
    {
        Ok(Some(session)) => session,
        Ok(None) => {
            return ui::error(
                "E8020",
                "the project has no registry; configure `[registry]` or `SEAL_REGISTRY_URL`",
            );
        }
        Err(error) => return ui::error(error.code(), error.message()),
    };
    let response = if args.claim {
        let proof = match &args.proof {
            Some(kind) => Some(NamespaceProof {
                kind: kind.clone(),
                value: args.proof_value.clone(),
            }),
            None => {
                return ui::error(
                    "E8093",
                    "`--claim` needs `--proof config`, `org`, `oidc`, or `dns`",
                );
            }
        };
        let request = OwnerRequest {
            namespace: args.namespace.clone(),
            action: "claim".to_owned(),
            owner: None,
            proof,
        };
        seal_registry::publish::claim(&session, &request)
    } else if args.add.is_some() || args.remove.is_some() {
        let (action, owner) = match (&args.add, &args.remove) {
            (Some(owner), None) => ("add", owner.clone()),
            (None, Some(owner)) => ("remove", owner.clone()),
            _ => {
                return ui::error("E8093", "choose exactly one of `--add` or `--remove`");
            }
        };
        send_owners(
            &session,
            &OwnerRequest {
                namespace: args.namespace.clone(),
                action: action.to_owned(),
                owner: Some(owner),
                proof: None,
            },
        )
    } else {
        send_owners(
            &session,
            &OwnerRequest {
                namespace: args.namespace.clone(),
                action: "list".to_owned(),
                owner: None,
                proof: None,
            },
        )
    };
    match response {
        Ok(response) => {
            print(cli, &response);
            ExitCode::SUCCESS
        }
        Err(error) => ui::error(error.code(), error.message()),
    }
}

fn print(cli: &Cli, response: &OwnerResponse) {
    ui::status(
        cli,
        &format!("{} (verified by {})", response.namespace, response.verified),
    );
    for owner in &response.owners {
        ui::detail(cli, &format!("  {owner}"));
    }
}
