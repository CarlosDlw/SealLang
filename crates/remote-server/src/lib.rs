//! A reference REAPI v2 server for Seal.
//!
//! The server stores immutable blobs and action results under a private root,
//! authenticates bearer tokens, serves a bounded CAS, executes actions in the
//! same sandbox as local generators, and persists its cache across restarts. It
//! is intended for integration tests and small self-hosted deployments, not as
//! a multi-tenant scheduler.

#![warn(missing_docs)]
// Tonic's generated service traits and streams use `tonic::Status` as their
// error type. Boxing it in internal helpers would only move the conversion to
// every call site without making the service boundary smaller.
#![allow(clippy::result_large_err)]

mod action_cache;
mod bytestream;
mod capabilities;
mod cas;
mod config;
mod error;
mod execution;
mod server;
mod state;
mod store;
mod worker;

pub use config::{
    DEFAULT_ACTION_TIMEOUT, DEFAULT_MAX_BATCH_BYTES, DEFAULT_MAX_BLOB_BYTES,
    DEFAULT_MAX_INPUT_BYTES, ServerConfig,
};
pub use error::{ServerError, ServerResult};
pub use server::RunningServer;
