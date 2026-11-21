//! Incremental query database for the compiler.
//!
//! A [`QueryDb`] memoizes pure computations keyed by a stable logical
//! [`QueryKey`]. Each result records the inputs and derived queries it read.
//! A later request validates those dependencies by content fingerprint, not by
//! modification time or absolute path. When a query recomputes to the same
//! fingerprint, its dependents stay valid: this is the red-green (backdating)
//! rule that stops an edit from propagating through the whole graph when the
//! semantic result did not change.
//!
//! The database never runs a computation while holding its lock, so a query
//! closure may read other queries. A thread-local active stack turns a
//! recursive dependency into [`QueryError::Cycle`] instead of a deadlock.
//! Independent queries can run concurrently; values are shared through
//! [`std::sync::Arc`] and must be `Send + Sync`.
//!
//! # Inputs
//!
//! Inputs are explicit named byte strings. Source text, manifests, features,
//! cfg values, toolchain descriptions, and declared environment values all use
//! the same mechanism. Reading the process environment is never part of the
//! database: a caller that wants an environment value must add it as an input.
//!
//! # Persistent cache
//!
//! [`DiskCache`] stores byte payloads for serializable results such as metadata
//! and code generation units. It is intentionally separate from [`QueryDb`]:
//! in-memory results can hold compiler arenas, while the disk cache only sees
//! bytes. The cache has a format version, a key check, a length check, and a
//! BLAKE3 checksum. A corrupt or old record is a miss, never a panic.
//!
//! # Example
//!
//! ```
//! use std::sync::Arc;
//! use seal_hash::stable_hash_of;
//! use seal_query::{
//!     InputKind, QueryDb, QueryError, QueryKey, QueryKind, QueryOutput,
//! };
//!
//! # fn main() -> Result<(), QueryError> {
//! let db = QueryDb::new();
//! let source = db.set_input(InputKind::Source, "src/main.sl", b"func main() {}")?;
//! let dependency = db.input_dependency(source)?;
//! let key = QueryKey::new(QueryKind::Tokens, "src/main.sl");
//! let first = db.query(key.clone(), [dependency.clone()], || {
//!     let text: Arc<str> = Arc::from("tokens");
//!     Ok(QueryOutput::new(text.clone(), stable_hash_of(text.as_ref())))
//! })?;
//! assert!(first.executed());
//!
//! let second = db.query::<Arc<str>, _>(key, [dependency], || unreachable!())?;
//! assert!(!second.executed());
//! assert_eq!(&***second.value(), "tokens");
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]

mod cache;
mod database;
mod error;
mod key;
mod output;
mod revision;

pub use cache::{CACHE_VERSION, CacheError, DiskCache, MAX_CACHE_PAYLOAD};
pub use database::{KindStats, QueryDb, QueryStats};
pub use error::QueryError;
pub use key::{InputId, InputKind, QueryKey, QueryKind};
pub use output::{QueryOutcome, QueryOutput};
pub use revision::{Dependency, InputStamp, QueryStamp, Revision};

/// The query-key domain tag is part of the persistent cache format.
///
/// Changing an existing tag requires bumping [`CACHE_VERSION`].
#[doc(hidden)]
pub fn key_digest(key: &QueryKey) -> seal_hash::Fingerprint {
    use seal_hash::StableHasher;
    let mut hasher = StableHasher::new();
    hasher.write_u8(key.kind().tag());
    hasher.write_str(key.name());
    hasher.finish()
}
