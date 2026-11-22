//! Optional REAPI v2 remote cache and execution.
//!
//! The compiler remains local-first: a remote endpoint is opt-in, every remote
//! failure is typed, and the caller decides whether to fall back or abort. The
//! client implements the REAPI services Seal needs over gRPC with BLAKE3
//! digests, bounded batches, Zstandard compression, ByteStream for large blobs,
//! bearer authentication, verified TLS, and digest checks on every transfer.

#![warn(missing_docs)]

pub mod auth;
pub mod config;
pub mod error;
pub mod types;

#[cfg(feature = "client")]
mod action_cache;
#[cfg(feature = "client")]
mod cas;
#[cfg(feature = "client")]
mod client;
#[cfg(feature = "client")]
mod execution;

#[cfg(feature = "client")]
pub use action_cache::RemoteCache;
#[cfg(feature = "client")]
pub use client::RemoteClient;

pub use auth::RemoteToken;
pub use config::{DEFAULT_INSTANCE, DEFAULT_MAX_BATCH_BYTES, RemoteConfig};
pub use error::{RemoteError, RemoteErrorKind, RemoteResult};
pub use types::{
    CacheAction, RemoteBlob, RemoteCapabilities, RemoteExecutionRequest, RemoteExecutionResult,
    RemoteFile, RemoteOutput, validate_relative_path,
};

/// The generated REAPI protocol types.
#[doc(hidden)]
#[allow(missing_docs, clippy::all)]
pub mod proto {
    pub mod google {
        pub mod longrunning {
            #[derive(Clone, PartialEq, ::prost::Message)]
            pub struct Operation {
                #[prost(string, tag = "1")]
                pub name: String,
                #[prost(message, optional, tag = "2")]
                pub metadata: Option<::prost_types::Any>,
                #[prost(bool, tag = "3")]
                pub done: bool,
                #[prost(message, optional, tag = "4")]
                pub error: Option<super::rpc::Status>,
                #[prost(message, optional, tag = "5")]
                pub response: Option<::prost_types::Any>,
            }
        }
        pub mod rpc {
            #[derive(Clone, PartialEq, ::prost::Message)]
            pub struct Status {
                #[prost(int32, tag = "1")]
                pub code: i32,
                #[prost(string, tag = "2")]
                pub message: String,
                #[prost(message, repeated, tag = "3")]
                pub details: Vec<::prost_types::Any>,
            }
        }
        pub mod bytestream {
            include!(concat!(env!("OUT_DIR"), "/google.bytestream.rs"));
        }
    }
    tonic::include_proto!("build.bazel.remote.execution.v2");
}
