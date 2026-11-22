//! The synchronous REAPI client.
//!
//! Tonic is asynchronous, so the client owns a small Tokio runtime and exposes
//! blocking methods to the compiler. Every unary request gets the configured
//! timeout and bearer metadata; the token never appears in command lines,
//! action messages, or logs.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::runtime::Runtime;
use tonic::metadata::{Ascii, MetadataValue};
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint};
use tonic::{Request, Response, Status};

use crate::config::RemoteConfig;
use crate::error::{RemoteError, RemoteResult};
use crate::proto;
use crate::types::RemoteCapabilities;

const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// A connected remote endpoint.
pub struct RemoteClient {
    config: RemoteConfig,
    channel: Channel,
    runtime: Runtime,
    authorization: Option<MetadataValue<Ascii>>,
    capabilities: std::sync::Mutex<Option<RemoteCapabilities>>,
}

impl RemoteClient {
    /// Opens a channel to the configured endpoint.
    ///
    /// The connection is established eagerly so an unreachable endpoint is
    /// reported before any action is attempted. TLS is verified with the host
    /// root store unless an explicit CA bundle is configured.
    pub fn connect(config: RemoteConfig) -> RemoteResult<Arc<Self>> {
        config.validate().map_err(RemoteError::configuration)?;
        let url = url::Url::parse(&config.endpoint)
            .map_err(|error| RemoteError::configuration(format!("invalid remote URL: {error}")))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(RemoteError::configuration(
                "the remote URL must use http or https",
            ));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(RemoteError::configuration(
                "credentials must be configured as a bearer token, not in the URL",
            ));
        }
        let mut endpoint = Endpoint::from_shared(config.endpoint.clone())
            .map_err(|error| {
                RemoteError::configuration(format!("invalid remote endpoint: {error}"))
            })?
            .connect_timeout(config.connect_timeout)
            .timeout(config.timeout)
            .tcp_nodelay(true);
        if url.scheme() == "https" {
            let mut tls = ClientTlsConfig::new();
            if let Some(path) = &config.ca_certificate {
                let pem = std::fs::read(path).map_err(|error| {
                    RemoteError::configuration(format!(
                        "cannot read the remote CA certificate `{}`: {error}",
                        path.display()
                    ))
                })?;
                tls = tls.ca_certificate(Certificate::from_pem(pem));
            }
            if let Some(domain) = &config.tls_domain {
                tls = tls.domain_name(domain.clone());
            }
            endpoint = endpoint.tls_config(tls).map_err(|error| {
                RemoteError::configuration(format!("cannot configure remote TLS: {error}"))
            })?;
        }
        let authorization = config
            .token
            .as_ref()
            .map(|token| {
                token
                    .header_value()
                    .parse::<MetadataValue<Ascii>>()
                    .map_err(|_| {
                        RemoteError::configuration(
                            "the remote token cannot be encoded as an authorization header",
                        )
                    })
            })
            .transpose()?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("seal-remote")
            .build()
            .map_err(|error| {
                RemoteError::configuration(format!("cannot start the remote runtime: {error}"))
            })?;
        let channel = runtime
            .block_on(endpoint.connect())
            .map_err(RemoteError::from)?;
        Ok(Arc::new(Self {
            config,
            channel,
            runtime,
            authorization,
            capabilities: std::sync::Mutex::new(None),
        }))
    }

    /// Returns the endpoint configuration.
    pub fn config(&self) -> &RemoteConfig {
        &self.config
    }

    /// Returns the instance name.
    pub fn instance_name(&self) -> &str {
        &self.config.instance_name
    }

    /// Returns whether remote execution is enabled.
    pub fn execution_enabled(&self) -> bool {
        self.config.execution
    }

    /// Returns the maximum batch size, bounded by the endpoint setting.
    pub fn max_batch_bytes(&self) -> usize {
        self.config.max_batch_bytes
    }

    /// Redacts the token from a message before it is logged or returned.
    pub fn redact(&self, text: &str) -> String {
        self.config
            .token
            .as_ref()
            .map_or_else(|| text.to_owned(), |token| token.redact(text))
    }

    /// Adds the bearer metadata to one request.
    pub fn request<T>(&self, message: T) -> Request<T> {
        let mut request = Request::new(message);
        if let Some(authorization) = &self.authorization {
            request
                .metadata_mut()
                .insert("authorization", authorization.clone());
        }
        request
    }

    /// Runs one unary request with the configured timeout.
    pub fn unary<T, F>(&self, future: F) -> RemoteResult<T>
    where
        F: Future<Output = Result<Response<T>, Status>>,
    {
        self.runtime.block_on(async {
            match tokio::time::timeout(self.config.timeout, future).await {
                Ok(Ok(response)) => Ok(response.into_inner()),
                Ok(Err(status)) => Err(RemoteError::from(status)),
                Err(_) => Err(RemoteError::timeout("the remote request timed out")),
            }
        })
    }

    /// Returns the channel for one generated client.
    pub fn channel(&self) -> Channel {
        self.channel.clone()
    }

    /// Returns the runtime used by streaming calls.
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// Returns the retry delay for an attempt.
    pub fn retry_delay(attempt: u32) -> Duration {
        Duration::from_millis(50_u64.saturating_mul(1_u64 << attempt.min(5)))
    }

    /// Reads and caches the server capabilities, verifying the features Seal
    /// needs on the first call.
    pub fn capabilities(&self) -> RemoteResult<RemoteCapabilities> {
        if let Some(capabilities) = self
            .capabilities
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
        {
            return Ok(capabilities);
        }
        let capabilities = self.fetch_capabilities()?;
        *self
            .capabilities
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(capabilities.clone());
        Ok(capabilities)
    }

    fn fetch_capabilities(&self) -> RemoteResult<RemoteCapabilities> {
        let mut client = proto::capabilities_client::CapabilitiesClient::new(self.channel())
            .max_decoding_message_size(MAX_MESSAGE_BYTES)
            .max_encoding_message_size(MAX_MESSAGE_BYTES);
        let request = proto::GetCapabilitiesRequest {
            instance_name: self.instance_name().to_owned(),
        };
        let response = self.unary(client.get_capabilities(self.request(request)))?;
        let cache = response.cache_capabilities.unwrap_or_default();
        let execution = response.execution_capabilities.unwrap_or_default();
        let blake3 = cache
            .digest_functions
            .contains(&(proto::digest_function::Value::Blake3 as i32));
        if !blake3 {
            return Err(RemoteError::unsupported(
                "the remote endpoint does not advertise BLAKE3 digests",
            ));
        }
        if self.execution_enabled() && !execution.exec_enabled {
            return Err(RemoteError::unsupported(
                "the remote endpoint does not enable execution",
            ));
        }
        let max_batch_bytes = if cache.max_batch_total_size_bytes <= 0 {
            self.max_batch_bytes()
        } else {
            self.max_batch_bytes()
                .min(cache.max_batch_total_size_bytes as usize)
        };
        Ok(RemoteCapabilities {
            max_batch_bytes: max_batch_bytes as i64,
            zstd: cache
                .supported_compressors
                .contains(&(proto::compressor::Value::Zstd as i32)),
            execution: execution.exec_enabled,
            blake3,
        })
    }
}

impl std::fmt::Debug for RemoteClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteClient")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}
