//! REAPI ActionCache.

use std::collections::BTreeSet;
use std::sync::Arc;

use prost::Message;
use seal_cas::Digest;
use seal_remote::proto;
use tonic::{Request, Response, Status};

use crate::state::{State, status};

/// The action-cache service.
pub(crate) struct ActionCacheService {
    pub(crate) state: Arc<State>,
}

#[tonic::async_trait]
impl proto::action_cache_server::ActionCache for ActionCacheService {
    async fn get_action_result(
        &self,
        request: Request<proto::GetActionResultRequest>,
    ) -> Result<Response<proto::ActionResult>, Status> {
        let message = request.get_ref();
        self.state
            .authorize(&request, &message.instance_name)
            .map_err(status)?;
        let (digest, _) = action_digest(message.action_digest.as_ref())?;
        let bytes = self
            .state
            .store
            .get_action(digest)?
            .ok_or_else(|| Status::not_found(format!("no action result for `{digest}`")))?;
        let result = proto::ActionResult::decode(bytes.as_slice())
            .map_err(|error| Status::internal(format!("the cached result is corrupt: {error}")))?;
        verify_outputs(&self.state, &result)?;
        Ok(Response::new(result))
    }

    async fn update_action_result(
        &self,
        request: Request<proto::UpdateActionResultRequest>,
    ) -> Result<Response<proto::ActionResult>, Status> {
        let message = request.get_ref();
        self.state
            .authorize(&request, &message.instance_name)
            .map_err(status)?;
        let (digest, size) = action_digest(message.action_digest.as_ref())?;
        let action_bytes = self.state.store.get_blob(digest)?;
        if action_bytes.len() as u64 != size {
            return Err(Status::invalid_argument(
                "the action digest size does not match the stored action",
            ));
        }
        let action = proto::Action::decode(action_bytes.as_slice())
            .map_err(|error| Status::invalid_argument(format!("invalid Action: {error}")))?;
        let command_digest = action
            .command_digest
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("the Action has no Command digest"))?;
        let (command, _) = action_digest(Some(command_digest))?;
        let command_bytes = self.state.store.get_blob(command).map_err(|_| {
            Status::failed_precondition("the Action Command is not present in the CAS")
        })?;
        proto::Command::decode(command_bytes.as_slice())
            .map_err(|error| Status::invalid_argument(format!("invalid Command: {error}")))?;
        let result = message
            .action_result
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("the update has no ActionResult"))?;
        verify_outputs(&self.state, result)?;
        let bytes = result.encode_to_vec();
        self.state.store.put_action(digest, &bytes)?;
        Ok(Response::new(result.clone()))
    }
}

fn action_digest(wire: Option<&proto::Digest>) -> Result<(Digest, u64), Status> {
    let wire = wire.ok_or_else(|| Status::invalid_argument("the request has no action digest"))?;
    if wire.size_bytes < 0 {
        return Err(Status::invalid_argument(
            "the action digest has a negative size",
        ));
    }
    let digest =
        Digest::parse(&wire.hash).map_err(|error| Status::invalid_argument(error.to_string()))?;
    Ok((digest, wire.size_bytes as u64))
}

pub(crate) fn verify_outputs(state: &State, result: &proto::ActionResult) -> Result<(), Status> {
    if result.exit_code < 0 {
        return Err(Status::invalid_argument(
            "an ActionResult cannot have a negative exit code",
        ));
    }
    if !result.output_directories.is_empty() || !result.output_symlinks.is_empty() {
        return Err(Status::unimplemented(
            "directory and symlink outputs are not served by this endpoint",
        ));
    }
    let mut paths = BTreeSet::new();
    for output in &result.output_files {
        seal_remote::validate_relative_path(&output.path)
            .map_err(|error| Status::invalid_argument(error.to_string()))?;
        if !paths.insert(&output.path) {
            return Err(Status::invalid_argument(format!(
                "the output `{}` appears twice",
                output.path
            )));
        }
        let wire = output
            .digest
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("an output has no digest"))?;
        if wire.size_bytes < 0 {
            return Err(Status::invalid_argument("an output has a negative size"));
        }
        let digest = Digest::parse(&wire.hash)
            .map_err(|error| Status::invalid_argument(error.to_string()))?;
        let bytes = state.store.get_blob(digest).map_err(|_| {
            Status::failed_precondition(format!(
                "the output `{}` is missing from the CAS",
                output.path
            ))
        })?;
        if bytes.len() as u64 != wire.size_bytes as u64 {
            return Err(Status::invalid_argument(format!(
                "the output `{}` has the wrong size",
                output.path
            )));
        }
        if !output.contents.is_empty()
            && (Digest::of_bytes(&output.contents) != digest || output.contents != bytes)
        {
            return Err(Status::invalid_argument(format!(
                "the inline output `{}` does not match the CAS",
                output.path
            )));
        }
    }
    Ok(())
}
