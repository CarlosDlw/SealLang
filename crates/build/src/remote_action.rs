//! Remote execution descriptors for compiler actions.
//!
//! Only actions whose inputs are data and whose command is a `sealc`
//! invocation can run on a generic worker. Link, native probing, and generator
//! execution stay local: they consume host toolchains, native libraries, and
//! package files that are not part of an action's declared input set.

use std::path::PathBuf;
use std::time::Duration;

use seal_remote::{RemoteBlob, RemoteExecutionRequest, RemoteOutput};

use crate::action::{Action, CodegenAction, FrontendAction};
use crate::context::ActionContext;
use crate::error::{BuildError, BuildResult};

const REMOTE_ACTION_TIMEOUT: Duration = Duration::from_secs(300);

pub(crate) fn spec(
    action: &Action,
    context: &ActionContext,
) -> BuildResult<Option<RemoteExecutionRequest>> {
    match action {
        Action::Frontend(frontend) => frontend_spec(frontend),
        Action::Codegen(codegen) => codegen_spec(codegen, context),
        _ => Ok(None),
    }
}

fn frontend_spec(action: &FrontendAction) -> BuildResult<Option<RemoteExecutionRequest>> {
    let mut plan = action.plan.clone();
    plan.output = Some(PathBuf::from("artifact.sealmono"));
    let mut inputs = Vec::new();
    for package in &mut plan.packages {
        package.root = PathBuf::from(".");
        for file in &mut package.files {
            let source = file.path.clone();
            let remote = crate::action::relative_name(&source, &action.project_root);
            if seal_remote::validate_relative_path(&remote).is_err() {
                // A source outside the project cannot be placed in a relative
                // input tree. Remote execution is best-effort, so this action
                // runs locally instead of failing an optional remote build.
                return Ok(None);
            }
            let bytes = std::fs::read(&source).map_err(|error| {
                BuildError::action(format!(
                    "cannot read `{}` for remote execution: {error}",
                    source.display()
                ))
            })?;
            inputs.push(RemoteBlob {
                path: remote.clone(),
                bytes,
            });
            file.path = PathBuf::from(remote);
        }
    }
    let plan = plan
        .to_json()
        .map_err(|error| BuildError::action(error.to_string()))?;
    inputs.push(RemoteBlob {
        path: "plan.json".to_owned(),
        bytes: plan.into_bytes(),
    });
    Ok(Some(RemoteExecutionRequest {
        arguments: vec![
            "sealc".to_owned(),
            "build".to_owned(),
            "--plan-file".to_owned(),
            "plan.json".to_owned(),
            "--emit=mono".to_owned(),
            "--link-metadata".to_owned(),
            "link.json".to_owned(),
        ],
        environment: Vec::new(),
        inputs,
        outputs: vec![
            RemoteOutput {
                path: "artifact.sealmono".to_owned(),
                name: "mono".to_owned(),
                executable: false,
            },
            RemoteOutput {
                path: "link.json".to_owned(),
                name: "link-metadata".to_owned(),
                executable: false,
            },
        ],
        platform: Vec::new(),
        timeout: REMOTE_ACTION_TIMEOUT,
    }))
}

fn codegen_spec(
    action: &CodegenAction,
    context: &ActionContext,
) -> BuildResult<Option<RemoteExecutionRequest>> {
    let mono = context.cache.cas().get(action.mono)?;
    Ok(Some(RemoteExecutionRequest {
        arguments: vec![
            "sealc".to_owned(),
            "codegen".to_owned(),
            "--mono".to_owned(),
            "mono".to_owned(),
            "--cgu".to_owned(),
            action.cgu.to_string(),
            "-o".to_owned(),
            "object".to_owned(),
        ],
        environment: Vec::new(),
        inputs: vec![RemoteBlob {
            path: "mono".to_owned(),
            bytes: mono,
        }],
        outputs: vec![RemoteOutput {
            path: "object".to_owned(),
            name: "object".to_owned(),
            executable: false,
        }],
        platform: Vec::new(),
        timeout: REMOTE_ACTION_TIMEOUT,
    }))
}
