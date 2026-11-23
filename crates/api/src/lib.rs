//! Public-API snapshots and semantic-version compatibility for Seal packages.
//!
//! The crate is intentionally independent from the compiler and the registry so
//! both `seal api`, `seal semver-check`, and the registry server classify the
//! same document with the same rules.

#![warn(missing_docs)]

pub mod error;
pub mod semver;
pub mod snapshot;

pub use error::{ApiError, ApiResult};
pub use semver::{ApiDiff, Bump, Change, check, enforce, required_bump};
pub use snapshot::{
    ApiDeprecation, ApiField, ApiImplementation, ApiItem, ApiLayout, ApiModule, ApiSnapshot,
    ApiVariant, FORMAT,
};
