//! Content-addressed storage and the local action cache.
//!
//! The store hashes every object with blake3 and writes it under `objects/`
//! using an atomic temporary-file plus rename protocol. Action results live in
//! `actions/` as JSON records that name their input and output digests; a record
//! is only usable while every referenced object is present and valid.
//!
//! The on-disk layout is private to this crate. Callers use [`Cas`] and
//! [`ActionCache`], so the sharding, file permissions, and lock protocol can
//! change without touching the build graph.
//!
//! # Security
//!
//! Cache directories are owner-only (`0700`) and objects are owner-readable and
//! owner-writable (`0600`). Environment values are stored as digests, never as
//! plaintext. Locks are `create_new` marker files with a stale timeout, which is
//! sufficient for the single-user local cache this phase defines.

#![warn(missing_docs)]

mod action;
mod cas;
mod digest;
mod error;

pub use action::{
    ActionCache, ActionInput, ActionOutput, ActionRecord, ActionStats, ActionVerifyReport,
};
pub use cas::{
    Cas, CasStats, CorruptObject, DEFAULT_CACHE_DIR, LockGuard, PruneReport, VerifyReport,
};
pub use digest::Digest;
pub use error::CasError;
