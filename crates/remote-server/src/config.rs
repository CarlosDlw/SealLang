//! Configuration for the reference remote server.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use seal_remote::RemoteToken;

/// Default maximum bytes in one batch request.
pub const DEFAULT_MAX_BATCH_BYTES: usize = 4 * 1024 * 1024;
/// Default maximum bytes in one CAS blob.
pub const DEFAULT_MAX_BLOB_BYTES: usize = 512 * 1024 * 1024;
/// Default maximum total input bytes for one action.
pub const DEFAULT_MAX_INPUT_BYTES: u64 = 256 * 1024 * 1024;
/// Default action timeout.
pub const DEFAULT_ACTION_TIMEOUT: Duration = Duration::from_secs(300);

/// One server configuration.
#[derive(Clone)]
pub struct ServerConfig {
    /// The persistent cache root.
    pub root: PathBuf,
    /// The REAPI instance name; empty accepts any instance.
    pub instance_name: String,
    /// An optional bearer token.
    pub token: Option<RemoteToken>,
    /// Whether remote execution is advertised and accepted.
    pub execution: bool,
    /// Maximum total bytes in one batch.
    pub max_batch_bytes: usize,
    /// Maximum bytes in one CAS blob.
    pub max_blob_bytes: usize,
    /// Maximum total input bytes for one action.
    pub max_input_bytes: u64,
    /// Maximum action timeout.
    pub max_action_timeout: Duration,
    /// Whether the portable sandbox fallback is allowed.
    pub allow_portable_sandbox: bool,
    /// An optional directory prepended when resolving the command program.
    pub tool_path: Option<PathBuf>,
    /// Optional PEM certificate and private key for TLS.
    pub tls: Option<(PathBuf, PathBuf)>,
}

impl ServerConfig {
    /// Validates the limits.
    pub fn validate(&self) -> Result<(), String> {
        if self.root.as_os_str().is_empty() {
            return Err("the remote cache root is empty".to_owned());
        }
        if self.instance_name.len() > 256 {
            return Err("the instance name exceeds 256 bytes".to_owned());
        }
        if self.max_batch_bytes == 0 || self.max_blob_bytes == 0 {
            return Err("batch and blob limits must be positive".to_owned());
        }
        if self.max_batch_bytes > self.max_blob_bytes {
            return Err("the batch limit cannot exceed the blob limit".to_owned());
        }
        if self.max_input_bytes == 0 || self.max_action_timeout.is_zero() {
            return Err("input and timeout limits must be positive".to_owned());
        }
        if let Some((cert, key)) = &self.tls
            && (cert.as_os_str().is_empty() || key.as_os_str().is_empty())
        {
            return Err("the TLS certificate and key paths cannot be empty".to_owned());
        }
        Ok(())
    }
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            root: PathBuf::from("seal-remote-cache"),
            instance_name: seal_remote::DEFAULT_INSTANCE.to_owned(),
            token: None,
            execution: true,
            max_batch_bytes: DEFAULT_MAX_BATCH_BYTES,
            max_blob_bytes: DEFAULT_MAX_BLOB_BYTES,
            max_input_bytes: DEFAULT_MAX_INPUT_BYTES,
            max_action_timeout: DEFAULT_ACTION_TIMEOUT,
            allow_portable_sandbox: false,
            tool_path: None,
            tls: None,
        }
    }
}

impl fmt::Debug for ServerConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerConfig")
            .field("root", &self.root)
            .field("instance_name", &self.instance_name)
            .field("token", &self.token)
            .field("execution", &self.execution)
            .field("max_batch_bytes", &self.max_batch_bytes)
            .field("max_blob_bytes", &self.max_blob_bytes)
            .field("max_input_bytes", &self.max_input_bytes)
            .field("max_action_timeout", &self.max_action_timeout)
            .field("allow_portable_sandbox", &self.allow_portable_sandbox)
            .field("tool_path", &self.tool_path)
            .field("tls", &self.tls)
            .finish()
    }
}
