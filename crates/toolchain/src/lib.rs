//! Pinned, checksum-verified toolchains with offline installation.
//!
//! A bundle is a directory with `seal-toolchain.toml` and the component files it
//! names. Installation verifies every regular file against the manifest's
//! SHA-256 tree digest before copying, then verifies the staged copy again. A
//! project pin is the same manifest at the project root; builds compare it with
//! the embedded compiler version and the requested target.

#![warn(missing_docs)]

mod error;
mod manifest;
mod store;
mod verify;

pub use error::ToolchainError;
pub use manifest::{MANIFEST_NAME, ToolchainManifest, validate_checksum};
pub use store::{
    InstalledToolchain, PIN_NAME, ToolchainStore, ensure_compatible, read_pin, write_pin,
};
pub use verify::{tree_checksum, verify_tree};

/// The compiler version embedded in this `seal` binary.
pub const EMBEDDED_VERSION: &str = env!("CARGO_PKG_VERSION");
