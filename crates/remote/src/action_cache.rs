//! Remote action-cache lookups and updates.
//!
//! The wire key is the digest of a canonical REAPI `Action` whose salt is the
//! local action key. Its `Command` commits the action kind, input paths and
//! digests, hashed environment, display command, and expected outputs. A remote
//! hit therefore cannot alias a different local action.

use std::sync::Arc;

use prost::Message;
use seal_cas::{ActionOutput, ActionRecord, Cas, Digest};

use crate::client::RemoteClient;
use crate::error::{RemoteError, RemoteErrorKind, RemoteResult};
use crate::proto;
use crate::types::{CacheAction, empty_digest, platform, to_proto_sized};

const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// A remote action cache bound to one local CAS.
#[derive(Clone, Debug)]
pub struct RemoteCache {
    client: Arc<RemoteClient>,
    local: Cas,
    platform: Vec<(String, String)>,
}

impl RemoteCache {
    /// Creates a cache that materializes hits into `local`.
    pub fn new(client: Arc<RemoteClient>, local: Cas) -> Self {
        let platform = client.config().platform.clone();
        Self {
            client,
            local,
            platform,
        }
    }

    /// Returns the client.
    pub fn client(&self) -> &Arc<RemoteClient> {
        &self.client
    }

    /// Looks up one action result, materializing every output first.
    pub fn lookup(&self, action: &CacheAction) -> RemoteResult<Option<ActionRecord>> {
        let (action_digest, action_size) = self.upload_action(action)?;
        let request = proto::GetActionResultRequest {
            instance_name: self.client.instance_name().to_owned(),
            action_digest: Some(to_proto_sized(action_digest, action_size)?),
            inline_stdout: false,
            inline_stderr: false,
            inline_output_files: Vec::new(),
            digest_function: proto::digest_function::Value::Blake3 as i32,
        };
        let result = match self.client.retry_unary(|| {
            let mut client =
                proto::action_cache_client::ActionCacheClient::new(self.client.channel())
                    .max_decoding_message_size(MAX_MESSAGE_BYTES)
                    .max_encoding_message_size(MAX_MESSAGE_BYTES);
            self.client
                .unary(client.get_action_result(self.client.request(request.clone())))
        }) {
            Ok(result) => result,
            Err(error) if error.kind() == RemoteErrorKind::MissingOutput => return Ok(None),
            Err(error) => return Err(error),
        };
        if !action.outputs.is_empty() {
            let missing: Vec<&str> = action
                .outputs
                .iter()
                .filter(|name| {
                    !result
                        .output_files
                        .iter()
                        .any(|output| &output.path == *name)
                })
                .map(String::as_str)
                .collect();
            if !missing.is_empty() {
                return Err(RemoteError::missing_output(format!(
                    "the remote result for `{}` is missing output(s): {}",
                    action.kind,
                    missing.join(", ")
                )));
            }
        }
        let mut outputs = Vec::with_capacity(result.output_files.len());
        for output in result.output_files {
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
                self.client.download_blob(parsed, size)?
            };
            if self.local.put_bytes(&bytes)? != parsed {
                return Err(RemoteError::integrity(
                    "the local CAS stored a different digest",
                ));
            }
            outputs.push(ActionOutput {
                name: output.path,
                digest: parsed,
                mode: if output.is_executable { 0o755 } else { 0o644 },
            });
        }
        if result.exit_code != 0 {
            return Err(RemoteError::action_failed(
                result.exit_code,
                "the cached remote action did not succeed",
            ));
        }
        let mut record = ActionRecord::new(action.kind.clone(), action.inputs.clone(), outputs)
            .with_command(action.command.clone());
        record.env = action.environment.clone();
        Ok(Some(record))
    }

    /// Stores one successful action result after uploading every output.
    pub fn store(&self, action: &CacheAction, record: &ActionRecord) -> RemoteResult<()> {
        let (action_digest, action_size) = self.upload_action(action)?;
        let mut files = Vec::with_capacity(record.outputs.len());
        for output in &record.outputs {
            let bytes = self.local.get(output.digest)?;
            self.client.upload_blob(output.digest, &bytes)?;
            files.push(proto::OutputFile {
                path: output.name.clone(),
                digest: Some(to_proto_sized(output.digest, bytes.len() as u64)?),
                is_executable: output.mode & 0o111 != 0,
                contents: Vec::new(),
            });
        }
        let result = proto::ActionResult {
            output_files: files,
            output_directories: Vec::new(),
            exit_code: 0,
            stdout_raw: Vec::new(),
            stdout_digest: None,
            stderr_raw: Vec::new(),
            stderr_digest: None,
            output_symlinks: Vec::new(),
        };
        let request = proto::UpdateActionResultRequest {
            instance_name: self.client.instance_name().to_owned(),
            action_digest: Some(to_proto_sized(action_digest, action_size)?),
            action_result: Some(result),
            results_cache_policy: None,
            digest_function: proto::digest_function::Value::Blake3 as i32,
        };
        self.client
            .retry_unary(|| {
                let mut client =
                    proto::action_cache_client::ActionCacheClient::new(self.client.channel())
                        .max_decoding_message_size(MAX_MESSAGE_BYTES)
                        .max_encoding_message_size(MAX_MESSAGE_BYTES);
                self.client
                    .unary(client.update_action_result(self.client.request(request.clone())))
            })
            .map(|_| ())
            .map_err(|error| match error.kind() {
                RemoteErrorKind::Authentication
                | RemoteErrorKind::Transport
                | RemoteErrorKind::Timeout => error,
                _ => RemoteError::cache_update(error.message().to_owned()),
            })
    }

    fn upload_action(&self, action: &CacheAction) -> RemoteResult<(Digest, u64)> {
        let mut arguments = vec![format!("kind:{}", action.kind)];
        let mut inputs = action.inputs.clone();
        inputs.sort_by(|left, right| left.path.cmp(&right.path));
        for input in &inputs {
            arguments.push(format!("input:{}={}", input.path, input.digest));
        }
        for (name, digest) in &action.environment {
            arguments.push(format!("env:{name}={digest}"));
        }
        for argument in &action.command {
            arguments.push(format!("cmd:{argument}"));
        }
        let mut outputs = action.outputs.clone();
        outputs.sort();
        for output in &outputs {
            arguments.push(format!("output:{output}"));
        }
        let command = proto::Command {
            arguments,
            environment_variables: Vec::new(),
            output_paths: outputs,
            working_directory: String::new(),
            output_node_properties: Vec::new(),
            output_directory_format: proto::command::OutputDirectoryFormat::TreeOnly as i32,
            ..Default::default()
        };
        let command_bytes = command.encode_to_vec();
        let command_digest = Digest::of_bytes(&command_bytes);
        self.client.upload_blob(command_digest, &command_bytes)?;
        let action_message = proto::Action {
            command_digest: Some(to_proto_sized(command_digest, command_bytes.len() as u64)?),
            input_root_digest: Some(empty_digest()),
            timeout: None,
            do_not_cache: false,
            salt: action.key.as_bytes().to_vec(),
            platform: Some(platform(&self.platform)),
        };
        let action_bytes = action_message.encode_to_vec();
        let action_digest = Digest::of_bytes(&action_bytes);
        self.client.upload_blob(action_digest, &action_bytes)?;
        Ok((action_digest, action_bytes.len() as u64))
    }
}
