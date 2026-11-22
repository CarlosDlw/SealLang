//! REAPI Execution.

use std::pin::Pin;
use std::sync::Arc;

use prost::Message;
use seal_cas::Digest;
use seal_remote::proto;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

use crate::action_cache::verify_outputs;
use crate::cas::{ok_status, wire_digest};
use crate::error::ServerError;
use crate::state::{State, status};
use crate::worker;

type OperationStream =
    Pin<Box<dyn Stream<Item = Result<proto::google::longrunning::Operation, Status>> + Send>>;

/// The execution service.
pub(crate) struct ExecutionService {
    pub(crate) state: Arc<State>,
}

#[tonic::async_trait]
impl proto::execution_server::Execution for ExecutionService {
    type ExecuteStream = OperationStream;
    type WaitExecutionStream = OperationStream;

    async fn execute(
        &self,
        request: Request<proto::ExecuteRequest>,
    ) -> Result<Response<Self::ExecuteStream>, Status> {
        let message = request.get_ref();
        self.state
            .authorize(&request, &message.instance_name)
            .map_err(status)?;
        if !self.state.config.execution {
            return Err(Status::unimplemented(
                "remote execution is disabled on this endpoint",
            ));
        }
        check_digest_function(message.digest_function)?;
        let (digest, size) = parse_action_digest(message.action_digest.as_ref())?;
        let action_bytes = self.state.store.get_blob(digest).map_err(status)?;
        if action_bytes.len() as u64 != size {
            return Err(Status::invalid_argument(
                "the action digest size does not match the stored action",
            ));
        }
        let action = proto::Action::decode(action_bytes.as_slice())
            .map_err(|error| Status::invalid_argument(format!("invalid Action: {error}")))?;
        check_platform(&action)?;
        let command_wire = action
            .command_digest
            .as_ref()
            .ok_or_else(|| Status::invalid_argument("the Action has no Command digest"))?;
        let (command_digest, command_size) = parse_action_digest(Some(command_wire))?;
        let command_bytes = self.state.store.get_blob(command_digest).map_err(status)?;
        if command_bytes.len() as u64 != command_size {
            return Err(Status::invalid_argument(
                "the Command digest size does not match the stored Command",
            ));
        }
        let command = proto::Command::decode(command_bytes.as_slice())
            .map_err(|error| Status::invalid_argument(format!("invalid Command: {error}")))?;
        if !message.skip_cache_lookup
            && !action.do_not_cache
            && let Some(cached) = self.state.store.get_action(digest).map_err(status)?
        {
            let result = proto::ActionResult::decode(cached.as_slice()).map_err(|error| {
                Status::internal(format!("the cached action result is corrupt: {error}"))
            })?;
            verify_outputs(&self.state, &result)?;
            let response = proto::ExecuteResponse {
                result: Some(result),
                cached_result: true,
                status: Some(ok_status()),
                message: String::new(),
            };
            return Ok(Response::new(operation_stream(
                &self.state,
                digest,
                size,
                response,
            )));
        }
        let state = Arc::clone(&self.state);
        let action_for_worker = action.clone();
        let command_for_worker = command.clone();
        let response = tokio::task::spawn_blocking(move || {
            worker::run(&state, &action_for_worker, &command_for_worker)
        })
        .await
        .map_err(|error| Status::internal(format!("the worker panicked: {error}")))?;
        let response = match response {
            Ok(result) => {
                if !action.do_not_cache {
                    let bytes = result.encode_to_vec();
                    self.state
                        .store
                        .put_action(digest, &bytes)
                        .map_err(status)?;
                }
                proto::ExecuteResponse {
                    result: Some(result),
                    cached_result: false,
                    status: Some(ok_status()),
                    message: String::new(),
                }
            }
            Err(error) if error.status() == tonic::Code::DeadlineExceeded => {
                proto::ExecuteResponse {
                    result: None,
                    cached_result: false,
                    status: Some(status_of(&error)),
                    message: error.to_string(),
                }
            }
            Err(error) => {
                return Err(status(error));
            }
        };
        Ok(Response::new(operation_stream(
            &self.state,
            digest,
            size,
            response,
        )))
    }

    async fn wait_execution(
        &self,
        request: Request<proto::WaitExecutionRequest>,
    ) -> Result<Response<Self::WaitExecutionStream>, Status> {
        let message = request.get_ref();
        self.state.authorize(&request, "").map_err(status)?;
        let operation = self
            .state
            .operations
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&message.name)
            .cloned()
            .ok_or_else(|| Status::not_found(format!("unknown operation `{}`", message.name)))?;
        Ok(Response::new(Box::pin(tokio_stream::once(Ok(operation)))))
    }
}

fn operation_stream(
    state: &State,
    digest: Digest,
    size: u64,
    response: proto::ExecuteResponse,
) -> OperationStream {
    let operation = make_operation(state, digest, size, response);
    state
        .operations
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(operation.name.clone(), operation.clone());
    Box::pin(tokio_stream::once(Ok(operation)))
}

fn make_operation(
    state: &State,
    digest: Digest,
    size: u64,
    response: proto::ExecuteResponse,
) -> proto::google::longrunning::Operation {
    let metadata = proto::ExecuteOperationMetadata {
        stage: proto::execution_stage::Value::Completed as i32,
        action_digest: Some(wire_digest(digest, size)),
        stdout_stream_name: String::new(),
        stderr_stream_name: String::new(),
        digest_function: proto::digest_function::Value::Blake3 as i32,
    };
    proto::google::longrunning::Operation {
        name: state.operation_name(),
        metadata: Some(prost_types::Any {
            type_url:
                "type.googleapis.com/build.bazel.remote.execution.v2.ExecuteOperationMetadata"
                    .to_owned(),
            value: metadata.encode_to_vec(),
        }),
        done: true,
        error: None,
        response: Some(prost_types::Any {
            type_url: "type.googleapis.com/build.bazel.remote.execution.v2.ExecuteResponse"
                .to_owned(),
            value: response.encode_to_vec(),
        }),
    }
}

fn check_digest_function(value: i32) -> Result<(), Status> {
    match proto::digest_function::Value::try_from(value)
        .unwrap_or(proto::digest_function::Value::Unknown)
    {
        proto::digest_function::Value::Unknown | proto::digest_function::Value::Blake3 => Ok(()),
        other => Err(Status::unimplemented(format!(
            "the digest function `{other:?}` is not supported"
        ))),
    }
}

fn parse_action_digest(wire: Option<&proto::Digest>) -> Result<(Digest, u64), Status> {
    let wire = wire.ok_or_else(|| Status::invalid_argument("the request has no action digest"))?;
    if wire.size_bytes < 0 {
        return Err(Status::invalid_argument(
            "the action digest has a negative size",
        ));
    }
    Ok((
        Digest::parse(&wire.hash).map_err(|error| Status::invalid_argument(error.to_string()))?,
        wire.size_bytes as u64,
    ))
}

fn check_platform(action: &proto::Action) -> Result<(), Status> {
    let Some(platform) = &action.platform else {
        return Ok(());
    };
    for property in &platform.properties {
        match property.name.as_str() {
            "os" if property.value != std::env::consts::OS => {
                return Err(Status::failed_precondition(format!(
                    "the action requires os `{}` but the worker is `{}`",
                    property.value,
                    std::env::consts::OS
                )));
            }
            "arch" if property.value != std::env::consts::ARCH => {
                return Err(Status::failed_precondition(format!(
                    "the action requires arch `{}` but the worker is `{}`",
                    property.value,
                    std::env::consts::ARCH
                )));
            }
            "tool" if property.value != format!("seal-{}", env!("CARGO_PKG_VERSION")) => {
                return Err(Status::failed_precondition(format!(
                    "the action requires tool `{}` but the worker is `seal-{}`",
                    property.value,
                    env!("CARGO_PKG_VERSION")
                )));
            }
            _ => {}
        }
    }
    Ok(())
}

fn status_of(error: &ServerError) -> proto::google::rpc::Status {
    let code = match error.status() {
        tonic::Code::DeadlineExceeded => 4,
        tonic::Code::InvalidArgument => 3,
        tonic::Code::NotFound => 5,
        _ => 13,
    };
    proto::google::rpc::Status {
        code,
        message: error.to_string(),
        details: Vec::new(),
    }
}
