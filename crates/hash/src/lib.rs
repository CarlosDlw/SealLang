//! Hashing infrastructure: fast in-memory maps and versioned stable hashing.
//!
//! Two hash families exist for two different jobs:
//!
//! - [`FxHasher`] for in-memory tables that never leave the process. Speed wins.
//! - [`StableHasher`] / [`StableHash`] for fingerprints that are persisted or
//!   compared across runs. Determinism wins.
//!
//! Using `std::collections::hash_map::DefaultHasher` for anything persisted is a
//! bug: its output is not specified and may change between releases.

#![warn(missing_docs)]

mod fx;
mod sip;
mod stable;

pub use fx::{FxHashMap, FxHashSet, FxHasher, FxIndexMap};
pub use stable::{FORMAT_VERSION, Fingerprint, StableHash, StableHasher, stable_hash_of};
