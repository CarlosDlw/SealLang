//! Remote endpoint configuration.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use crate::auth::RemoteToken;

/// The default REAPI instance name used by Seal.
pub const DEFAULT_INSTANCE: &str = "seal";

/// The default maximum size of one batch request.
pub const DEFAULT_MAX_BATCH_BYTES: usize = 4 * 1024 * 1024;

/// One remote endpoint.
#[derive(Clone)]
pub struct RemoteConfig {
    /// The gRPC endpoint, either `http://` or `https://`.
    pub endpoint: String,
    /// The REAPI instance name.
    pub instance_name: String,
    /// An optional bearer token.
    pub token: Option<RemoteToken>,
    /// An optional PEM certificate authority bundle for `https`.
    pub ca_certificate: Option<PathBuf>,
    /// The TLS server name when it differs from the URL host.
    pub tls_domain: Option<String>,
    /// The connection timeout.
    pub connect_timeout: Duration,
    /// The timeout of one unary request or the overall execution.
    pub timeout: Duration,
    /// Retries for transient, idempotent requests.
    pub retries: u32,
    /// The maximum bytes in one batch request.
    pub max_batch_bytes: usize,
    /// Whether remote execution is enabled.
    pub execution: bool,
    /// Whether remote failures abort the build instead of falling back.
    pub require_remote: bool,
    /// Platform properties sent with actions.
    pub platform: Vec<(String, String)>,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            instance_name: DEFAULT_INSTANCE.to_owned(),
            token: None,
            ca_certificate: None,
            tls_domain: None,
            connect_timeout: Duration::from_secs(10),
            timeout: Duration::from_secs(300),
            retries: 2,
            max_batch_bytes: DEFAULT_MAX_BATCH_BYTES,
            execution: false,
            require_remote: false,
            platform: default_platform(),
        }
    }
}

impl RemoteConfig {
    /// Returns the platform properties for this host.
    pub fn host_platform() -> Vec<(String, String)> {
        default_platform()
    }

    /// Validates limits before a connection is opened.
    pub fn validate(&self) -> Result<(), String> {
        if self.endpoint.trim().is_empty() {
            return Err("the remote endpoint is empty".to_owned());
        }
        if self.instance_name.len() > 256 {
            return Err("the remote instance name exceeds 256 bytes".to_owned());
        }
        if self.max_batch_bytes == 0 {
            return Err("the remote batch limit must be at least one byte".to_owned());
        }
        if self.connect_timeout.is_zero() {
            return Err("the remote connect timeout must be positive".to_owned());
        }
        if self.timeout.is_zero() {
            return Err("the remote timeout must be positive".to_owned());
        }
        if self.retries > 10 {
            return Err("the remote retry limit cannot exceed 10".to_owned());
        }
        for (name, value) in &self.platform {
            if name.is_empty() || value.is_empty() {
                return Err("remote platform property names and values cannot be empty".to_owned());
            }
        }
        Ok(())
    }
}

impl fmt::Debug for RemoteConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteConfig")
            .field("endpoint", &self.endpoint)
            .field("instance_name", &self.instance_name)
            .field("token", &self.token)
            .field("ca_certificate", &self.ca_certificate)
            .field("tls_domain", &self.tls_domain)
            .field("connect_timeout", &self.connect_timeout)
            .field("timeout", &self.timeout)
            .field("retries", &self.retries)
            .field("max_batch_bytes", &self.max_batch_bytes)
            .field("execution", &self.execution)
            .field("require_remote", &self.require_remote)
            .field("platform", &self.platform)
            .finish()
    }
}

fn default_platform() -> Vec<(String, String)> {
    vec![
        ("os".to_owned(), std::env::consts::OS.to_owned()),
        ("arch".to_owned(), std::env::consts::ARCH.to_owned()),
        (
            "tool".to_owned(),
            format!("seal-{}", env!("CARGO_PKG_VERSION")),
        ),
    ]
}
