//! A reference Seal registry server.
//!
//! The server publishes immutable package versions with namespaces, Ed25519
//! provenance, TUF metadata, and an append-only transparency log. It never
//! executes package code: an archive is decompressed with the client's bounded
//! safe extractor, its manifest is parsed, and only files are stored.

#![warn(missing_docs)]

pub mod auth;
pub mod config;
pub mod dns;
pub mod error;
pub mod http;
pub mod oidc;
pub mod publish;
pub mod server;
pub mod state;
pub mod store;
pub mod tuf;

pub use config::ServerConfig;
pub use error::{ServerError, ServerResult};
pub use server::{RunningServer, Server};
pub use store::Store;
