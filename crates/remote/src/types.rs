//! Public remote types and wire conversions.

use std::collections::BTreeMap;
use std::time::Duration;

use seal_cas::{ActionInput, Digest};

use crate::error::{RemoteError, RemoteResult};
use crate::proto;

/// One action-cache entry description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheAction {
    /// The local action key.
    pub key: Digest,
    /// The action kind.
    pub kind: String,
    /// The display command.
    pub command: Vec<String>,
    /// The input digests.
    pub inputs: Vec<ActionInput>,
    /// The hashed environment.
    pub environment: BTreeMap<String, Digest>,
    /// The output names the caller expects.
    pub outputs: Vec<String>,
}

/// One input blob for remote execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteBlob {
    /// The path relative to the input root.
    pub path: String,
    /// The file contents.
    pub bytes: Vec<u8>,
}

/// One output path requested from remote execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteOutput {
    /// The path relative to the working directory.
    pub path: String,
    /// The logical output name.
    pub name: String,
    /// Whether the file is executable.
    pub executable: bool,
}

/// One command to execute remotely.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteExecutionRequest {
    /// The command and its arguments.
    pub arguments: Vec<String>,
    /// Environment variables; values must never contain a secret.
    pub environment: Vec<(String, String)>,
    /// Input files.
    pub inputs: Vec<RemoteBlob>,
    /// Requested outputs.
    pub outputs: Vec<RemoteOutput>,
    /// Platform requirements merged with the endpoint defaults.
    pub platform: Vec<(String, String)>,
    /// The action timeout.
    pub timeout: Duration,
}

impl RemoteExecutionRequest {
    /// Validates every path, duplicate, and argument.
    pub fn validate(&self) -> RemoteResult<()> {
        if self.arguments.is_empty() || self.arguments[0].is_empty() {
            return Err(RemoteError::configuration(
                "a remote command needs a program argument",
            ));
        }
        if self.timeout.is_zero() {
            return Err(RemoteError::configuration(
                "a remote command needs a positive timeout",
            ));
        }
        let mut paths = BTreeMap::new();
        for input in &self.inputs {
            validate_relative_path(&input.path)?;
            if paths.insert(input.path.clone(), ()).is_some() {
                return Err(RemoteError::configuration(format!(
                    "the remote input `{}` appears twice",
                    input.path
                )));
            }
        }
        let mut outputs = BTreeMap::new();
        for output in &self.outputs {
            validate_relative_path(&output.path)?;
            if outputs.insert(output.path.clone(), ()).is_some() {
                return Err(RemoteError::configuration(format!(
                    "the remote output `{}` appears twice",
                    output.path
                )));
            }
        }
        for (name, value) in &self.environment {
            if name.is_empty() || name.contains(['=', '\0']) || value.contains('\0') {
                return Err(RemoteError::configuration(format!(
                    "the remote environment variable `{name}` is invalid"
                )));
            }
        }
        Ok(())
    }

    /// Returns the requested output names.
    pub fn output_names(&self) -> Vec<String> {
        self.outputs
            .iter()
            .map(|output| output.name.clone())
            .collect()
    }
}

/// One output file returned by remote execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFile {
    /// The path relative to the working directory.
    pub path: String,
    /// The logical output name.
    pub name: String,
    /// The file contents.
    pub bytes: Vec<u8>,
    /// Whether the file is executable.
    pub executable: bool,
}

/// The result of remote execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteExecutionResult {
    /// The returned files.
    pub files: Vec<RemoteFile>,
    /// Captured standard output.
    pub stdout: Vec<u8>,
    /// Captured standard error.
    pub stderr: Vec<u8>,
    /// The process exit code.
    pub exit_code: i32,
}

/// Remote server capabilities important to the client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteCapabilities {
    /// The maximum total size of one batch.
    pub max_batch_bytes: i64,
    /// Whether the server supports Zstandard batch compression.
    pub zstd: bool,
    /// Whether execution is enabled.
    pub execution: bool,
    /// Whether BLAKE3 is supported.
    pub blake3: bool,
}

/// Converts a digest and known size to its REAPI form.
pub(crate) fn to_proto_sized(digest: Digest, size: u64) -> RemoteResult<proto::Digest> {
    let size = i64::try_from(size)
        .map_err(|_| RemoteError::configuration("a blob exceeds the REAPI 64-bit size limit"))?;
    Ok(proto::Digest {
        hash: digest.to_hex(),
        size_bytes: size,
    })
}

/// Parses a wire digest after checking its size.
pub(crate) fn from_proto(
    digest: &proto::Digest,
    expected_size: Option<u64>,
) -> RemoteResult<Digest> {
    if digest.size_bytes < 0 {
        return Err(RemoteError::integrity(
            "the server returned a negative blob size",
        ));
    }
    if let Some(size) = expected_size
        && digest.size_bytes as u64 != size
    {
        return Err(RemoteError::integrity(format!(
            "the server returned size {} for a blob of {size} bytes",
            digest.size_bytes
        )));
    }
    Digest::parse(&digest.hash).map_err(RemoteError::from)
}

/// Returns the digest of the empty blob.
pub(crate) fn empty_digest() -> proto::Digest {
    to_proto_sized(Digest::of_bytes(b""), 0).expect("zero fits in i64")
}

/// Converts platform properties into the protobuf message.
pub(crate) fn platform(properties: &[(String, String)]) -> proto::Platform {
    let mut properties = properties.to_vec();
    properties.sort();
    properties.dedup();
    proto::Platform {
        properties: properties
            .into_iter()
            .map(|(name, value)| proto::platform::Property { name, value })
            .collect(),
    }
}

/// Validates that `path` is a normalized, relative path.
pub fn validate_relative_path(path: &str) -> RemoteResult<()> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains('\0')
        || path.ends_with('/')
        || path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(RemoteError::configuration(format!(
            "the remote path `{path}` must be a normalized relative path"
        )));
    }
    Ok(())
}

/// Returns a stable error for an unexpected output path.
pub(crate) fn unexpected_output(path: &str) -> RemoteError {
    RemoteError::protocol(format!(
        "the server returned an output path not requested by the action: `{path}`"
    ))
}
