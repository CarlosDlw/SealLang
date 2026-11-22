//! Remote execution.
//!
//! Inputs are uploaded as a canonical REAPI directory tree, the command and
//! action are stored in the CAS, and the client consumes the execution stream
//! until the operation completes. Outputs and logs are verified before they are
//! returned to the build.

use std::collections::BTreeMap;

use prost::Message;
use prost_types::Duration as ProtoDuration;
use seal_cas::Digest;

use crate::client::RemoteClient;
use crate::error::{RemoteError, RemoteResult};
use crate::proto;
use crate::types::{
    RemoteBlob, RemoteExecutionRequest, RemoteExecutionResult, RemoteFile, to_proto_sized,
};

const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

impl RemoteClient {
    /// Executes one command and returns its verified outputs and logs.
    pub fn execute(&self, request: &RemoteExecutionRequest) -> RemoteResult<RemoteExecutionResult> {
        request.validate()?;
        let mut platform = self.config().platform.clone();
        platform.extend(request.platform.iter().cloned());
        let input_root = self.upload_input_tree(&request.inputs)?;
        let mut environment = request.environment.clone();
        environment.sort();
        environment.dedup();
        let mut output_paths: Vec<String> = request
            .outputs
            .iter()
            .map(|output| output.path.clone())
            .collect();
        output_paths.sort();
        output_paths.dedup();
        let command = proto::Command {
            arguments: request.arguments.clone(),
            environment_variables: environment
                .into_iter()
                .map(|(name, value)| proto::command::EnvironmentVariable { name, value })
                .collect(),
            output_paths,
            working_directory: String::new(),
            output_node_properties: Vec::new(),
            output_directory_format: proto::command::OutputDirectoryFormat::TreeOnly as i32,
            ..Default::default()
        };
        let command_bytes = command.encode_to_vec();
        let command_digest = Digest::of_bytes(&command_bytes);
        self.upload_blob(command_digest, &command_bytes)?;
        let action = proto::Action {
            command_digest: Some(to_proto_sized(command_digest, command_bytes.len() as u64)?),
            input_root_digest: Some(input_root),
            timeout: Some(ProtoDuration {
                seconds: i64::try_from(request.timeout.as_secs()).unwrap_or(i64::MAX),
                nanos: request.timeout.subsec_nanos() as i32,
            }),
            do_not_cache: false,
            salt: Vec::new(),
            platform: Some(crate::types::platform(&platform)),
        };
        let action_bytes = action.encode_to_vec();
        let action_digest = Digest::of_bytes(&action_bytes);
        self.upload_blob(action_digest, &action_bytes)?;

        let response = self.run_execution(action_digest, action_bytes.len() as u64, request)?;
        self.decode_result(response, request)
    }

    fn run_execution(
        &self,
        action_digest: Digest,
        action_size: u64,
        request: &RemoteExecutionRequest,
    ) -> RemoteResult<proto::ExecuteResponse> {
        let overall = self.config().timeout.max(request.timeout);
        self.runtime().block_on(async {
            tokio::time::timeout(overall, async {
                let mut client = proto::execution_client::ExecutionClient::new(self.channel())
                    .max_decoding_message_size(MAX_MESSAGE_BYTES)
                    .max_encoding_message_size(MAX_MESSAGE_BYTES);
                let execute = self.request(proto::ExecuteRequest {
                    instance_name: self.instance_name().to_owned(),
                    skip_cache_lookup: false,
                    action_digest: Some(to_proto_sized(action_digest, action_size)?),
                    execution_policy: None,
                    results_cache_policy: None,
                    digest_function: proto::digest_function::Value::Blake3 as i32,
                    inline_stdout: true,
                    inline_stderr: true,
                    inline_output_files: Vec::new(),
                });
                let mut stream = client.execute(execute).await?.into_inner();
                let mut operation_name = String::new();
                while let Some(operation) = stream.message().await? {
                    if operation.name.is_empty() {
                        return Err(RemoteError::protocol(
                            "the execution stream returned an operation without a name",
                        ));
                    }
                    operation_name = operation.name.clone();
                    if operation.done {
                        return decode_operation(operation);
                    }
                }
                if operation_name.is_empty() {
                    return Err(RemoteError::protocol(
                        "the execution stream ended without an operation",
                    ));
                }
                let wait = self.request(proto::WaitExecutionRequest {
                    name: operation_name,
                });
                let mut stream = client.wait_execution(wait).await?.into_inner();
                while let Some(operation) = stream.message().await? {
                    if operation.done {
                        return decode_operation(operation);
                    }
                }
                Err(RemoteError::protocol(
                    "the execution stream ended before the action completed",
                ))
            })
            .await
            .map_err(|_| RemoteError::timeout("the remote execution timed out"))?
        })
    }

    fn decode_result(
        &self,
        response: proto::ExecuteResponse,
        request: &RemoteExecutionRequest,
    ) -> RemoteResult<RemoteExecutionResult> {
        let status = response.status.unwrap_or_default();
        if status.code == 4 {
            return Err(RemoteError::timeout(format!(
                "remote execution failed: {}",
                status.message
            )));
        }
        if status.code != 0 {
            return Err(RemoteError::action_failed(
                status.code,
                format!("remote execution failed: {}", status.message),
            ));
        }
        let result = response.result.ok_or_else(|| {
            RemoteError::protocol("the remote execution returned no action result")
        })?;
        let expected: BTreeMap<String, String> = request
            .outputs
            .iter()
            .map(|output| (output.path.clone(), output.name.clone()))
            .collect();
        let mut files = Vec::with_capacity(result.output_files.len());
        for output in result.output_files {
            let Some(name) = expected.get(&output.path).cloned() else {
                return Err(crate::types::unexpected_output(&output.path));
            };
            let digest = output.digest.ok_or_else(|| {
                RemoteError::integrity(format!("the remote output `{}` has no digest", output.path))
            })?;
            let size = u64::try_from(digest.size_bytes)
                .map_err(|_| RemoteError::integrity("the remote output has a negative size"))?;
            let parsed = Digest::parse(&digest.hash)?;
            let bytes = if !output.contents.is_empty() {
                if Digest::of_bytes(&output.contents) != parsed {
                    return Err(RemoteError::integrity(format!(
                        "the inline remote output `{}` does not match its digest",
                        output.path
                    )));
                }
                output.contents
            } else {
                self.download_blob(parsed, size)?
            };
            files.push(RemoteFile {
                path: output.path,
                name,
                bytes,
                executable: output.is_executable,
            });
        }
        for output in &request.outputs {
            if !files.iter().any(|file| file.path == output.path) {
                return Err(RemoteError::missing_output(format!(
                    "the remote action did not produce `{}`",
                    output.path
                )));
            }
        }
        let stdout = self.read_log(result.stdout_raw, result.stdout_digest.as_ref())?;
        let stderr = self.read_log(result.stderr_raw, result.stderr_digest.as_ref())?;
        Ok(RemoteExecutionResult {
            files,
            stdout,
            stderr,
            exit_code: result.exit_code,
        })
    }

    fn read_log(&self, inline: Vec<u8>, digest: Option<&proto::Digest>) -> RemoteResult<Vec<u8>> {
        if !inline.is_empty() {
            return Ok(inline);
        }
        let Some(digest) = digest else {
            return Ok(Vec::new());
        };
        let size = u64::try_from(digest.size_bytes)
            .map_err(|_| RemoteError::integrity("a remote log has a negative size"))?;
        self.download_blob(Digest::parse(&digest.hash)?, size)
    }

    fn upload_input_tree(&self, inputs: &[RemoteBlob]) -> RemoteResult<proto::Digest> {
        let mut root = TreeNode::default();
        for input in inputs {
            root.insert(&input.path, input.bytes.clone())?;
        }
        let (digest, size) = self.upload_directory(&root)?;
        to_proto_sized(digest, size)
    }

    fn upload_directory(&self, node: &TreeNode) -> RemoteResult<(Digest, u64)> {
        let mut files = Vec::with_capacity(node.files.len());
        for (name, bytes) in &node.files {
            let digest = Digest::of_bytes(bytes);
            self.upload_blob(digest, bytes)?;
            files.push(proto::FileNode {
                name: name.clone(),
                digest: Some(to_proto_sized(digest, bytes.len() as u64)?),
                is_executable: false,
            });
        }
        let mut directories = Vec::with_capacity(node.directories.len());
        for (name, child) in &node.directories {
            let (digest, size) = self.upload_directory(child)?;
            directories.push(proto::DirectoryNode {
                name: name.clone(),
                digest: Some(to_proto_sized(digest, size)?),
            });
        }
        let directory = proto::Directory {
            files,
            directories,
            symlinks: Vec::new(),
        };
        let bytes = directory.encode_to_vec();
        let digest = Digest::of_bytes(&bytes);
        self.upload_blob(digest, &bytes)?;
        Ok((digest, bytes.len() as u64))
    }
}

#[derive(Default)]
struct TreeNode {
    files: BTreeMap<String, Vec<u8>>,
    directories: BTreeMap<String, TreeNode>,
}

impl TreeNode {
    fn insert(&mut self, path: &str, bytes: Vec<u8>) -> RemoteResult<()> {
        let mut segments = path.split('/').peekable();
        let mut node = self;
        while let Some(segment) = segments.next() {
            if segments.peek().is_none() {
                if node.directories.contains_key(segment) {
                    return Err(RemoteError::configuration(format!(
                        "the remote input `{path}` is both a file and a directory"
                    )));
                }
                if node.files.insert(segment.to_owned(), bytes).is_some() {
                    return Err(RemoteError::configuration(format!(
                        "the remote input `{path}` appears twice"
                    )));
                }
                return Ok(());
            }
            node = node.directories.entry(segment.to_owned()).or_default();
        }
        Err(RemoteError::configuration("an empty remote input path"))
    }
}

fn decode_operation(
    operation: proto::google::longrunning::Operation,
) -> RemoteResult<proto::ExecuteResponse> {
    if let Some(error) = operation.error
        && error.code != 0
    {
        if error.code == 4 {
            return Err(RemoteError::timeout(format!(
                "remote operation failed: {}",
                error.message
            )));
        }
        return Err(RemoteError::action_failed(
            error.code,
            format!("remote operation failed: {}", error.message),
        ));
    }
    let response = operation
        .response
        .ok_or_else(|| RemoteError::protocol("the completed operation has no ExecuteResponse"))?;
    if !response.type_url.ends_with("ExecuteResponse") {
        return Err(RemoteError::protocol(format!(
            "the operation returned unexpected response type `{}`",
            response.type_url
        )));
    }
    proto::ExecuteResponse::decode(response.value.as_slice())
        .map_err(|error| RemoteError::protocol(format!("cannot decode ExecuteResponse: {error}")))
}
