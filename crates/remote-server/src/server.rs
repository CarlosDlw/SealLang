//! The reference server lifecycle.

use std::net::SocketAddr;
use std::sync::Arc;
use std::thread::JoinHandle;

use seal_remote::proto;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::transport::{Identity, Server, ServerTlsConfig};

use crate::action_cache::ActionCacheService;
use crate::bytestream::ByteStreamService;
use crate::capabilities::CapabilitiesService;
use crate::cas::CasService;
use crate::config::ServerConfig;
use crate::error::{ServerError, ServerResult};
use crate::execution::ExecutionService;
use crate::state::State;

const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// A running server.
pub struct RunningServer {
    endpoint: String,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    handle: Option<JoinHandle<()>>,
}

impl RunningServer {
    /// Starts a server on `address`.
    pub fn start(config: ServerConfig, address: SocketAddr) -> ServerResult<Self> {
        config.validate().map_err(ServerError::invalid)?;
        let tls = config.tls.is_some();
        let state = Arc::new(State::open(config)?);
        let (ready, receiver) = std::sync::mpsc::sync_channel(1);
        let (shutdown, shutdown_receiver) = tokio::sync::oneshot::channel();
        let handle = std::thread::Builder::new()
            .name("seal-remote-server".to_owned())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready.send(Err(format!("cannot start the server runtime: {error}")));
                        return;
                    }
                };
                runtime.block_on(async move {
                    let listener = match tokio::net::TcpListener::bind(address).await {
                        Ok(listener) => listener,
                        Err(error) => {
                            let _ = ready.send(Err(format!("cannot bind {address}: {error}")));
                            return;
                        }
                    };
                    let local = match listener.local_addr() {
                        Ok(local) => local,
                        Err(error) => {
                            let _ = ready.send(Err(format!("cannot read the bound address: {error}")));
                            return;
                        }
                    };
                    let builder = Server::builder();
                    let mut builder = if tls {
                        match server_tls(&state.config) {
                            Ok(configured) => match builder.tls_config(configured) {
                                Ok(builder) => builder,
                                Err(error) => {
                                    let _ = ready.send(Err(format!(
                                        "cannot configure server TLS: {error}"
                                    )));
                                    return;
                                }
                            },
                            Err(error) => {
                                let _ = ready.send(Err(error.to_string()));
                                return;
                            }
                        }
                    } else {
                        builder
                    };
                    let service = builder
                        .add_service(
                            proto::execution_server::ExecutionServer::new(ExecutionService {
                                state: Arc::clone(&state),
                            })
                            .max_decoding_message_size(MAX_MESSAGE_BYTES)
                            .max_encoding_message_size(MAX_MESSAGE_BYTES),
                        )
                        .add_service(
                            proto::action_cache_server::ActionCacheServer::new(
                                ActionCacheService {
                                    state: Arc::clone(&state),
                                },
                            )
                            .max_decoding_message_size(MAX_MESSAGE_BYTES)
                            .max_encoding_message_size(MAX_MESSAGE_BYTES),
                        )
                        .add_service(
                            proto::content_addressable_storage_server::ContentAddressableStorageServer::new(
                                CasService {
                                    state: Arc::clone(&state),
                                },
                            )
                            .max_decoding_message_size(MAX_MESSAGE_BYTES)
                            .max_encoding_message_size(MAX_MESSAGE_BYTES),
                        )
                        .add_service(
                            proto::capabilities_server::CapabilitiesServer::new(
                                CapabilitiesService {
                                    state: Arc::clone(&state),
                                },
                            )
                            .max_decoding_message_size(MAX_MESSAGE_BYTES)
                            .max_encoding_message_size(MAX_MESSAGE_BYTES),
                        )
                        .add_service(
                            proto::google::bytestream::byte_stream_server::ByteStreamServer::new(
                                ByteStreamService { state },
                            )
                            .max_decoding_message_size(MAX_MESSAGE_BYTES)
                            .max_encoding_message_size(MAX_MESSAGE_BYTES),
                        );
                    let _ = ready.send(Ok(local));
                    let incoming = TcpListenerStream::new(listener);
                    let _ = service
                        .serve_with_incoming_shutdown(incoming, async {
                            let _ = shutdown_receiver.await;
                        })
                        .await;
                });
            })
            .map_err(|error| ServerError::internal(format!("cannot start the server: {error}")))?;
        let address = receiver
            .recv()
            .map_err(|_| ServerError::internal("the server did not start"))?
            .map_err(ServerError::internal)?;
        let scheme = if tls { "https" } else { "http" };
        Ok(Self {
            endpoint: format!("{scheme}://{address}"),
            shutdown: Some(shutdown),
            handle: Some(handle),
        })
    }

    /// Returns the endpoint URL.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Requests a graceful shutdown and joins the server thread.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn server_tls(config: &ServerConfig) -> ServerResult<ServerTlsConfig> {
    let (certificate, key) = config
        .tls
        .as_ref()
        .ok_or_else(|| ServerError::internal("TLS is not configured"))?;
    let certificate = std::fs::read(certificate).map_err(|error| {
        ServerError::internal(format!("cannot read the TLS certificate: {error}"))
    })?;
    let key = std::fs::read(key)
        .map_err(|error| ServerError::internal(format!("cannot read the TLS key: {error}")))?;
    Ok(ServerTlsConfig::new().identity(Identity::from_pem(certificate, key)))
}
