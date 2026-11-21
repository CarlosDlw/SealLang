//! The `.sealmono` frontend artifact and its versioned schema.
//!
//! The frontend runs analysis, monomorphization, and IR construction once. It
//! partitions the resulting module into code generation units and stores their
//! LLVM bitcode in one container. A `codegen` action reads one unit and produces
//! one object without lowering types or MIR again.
//!
//! The container starts with a fixed magic, a format version, and a JSON header;
//! each module is an independent byte range. Every byte range has a size and a
//! blake3 digest in the header, so a truncated or corrupted artifact fails with
//! [`MetadataError`] instead of reaching LLVM.
//!
//! ```
//! use seal_metadata::{MonoManifest, MonoModule, write_archive, read_archive};
//!
//! let mut manifest = MonoManifest::new("x86_64-unknown-linux-gnu", 1);
//! manifest.modules.push(MonoModule {
//!     cgu: 0,
//!     name: "cgu0".to_owned(),
//!     digest: String::new(),
//!     size: 0,
//!     functions: 1,
//! });
//! let bytes = write_archive(&manifest, &[b"module".to_vec()]).expect("archive");
//! let archive = read_archive(&bytes).expect("valid archive");
//! assert_eq!(archive.modules(), &[b"module".to_vec()]);
//! ```

#![warn(missing_docs)]

mod error;
mod format;
mod schema;
mod validate;

pub use error::MetadataError;
pub use format::{
    Archive, MAGIC, MAX_ARCHIVE_BYTES, read_archive, read_archive_file, write_archive,
    write_archive_file,
};
pub use schema::{
    FORMAT_VERSION, MonoLayout, MonoManifest, MonoModule, MonoProfile, MonoSourceFile, MonoSummary,
};

/// Returns the lowercase blake3 digest of `bytes`.
#[must_use]
pub fn digest_bytes(bytes: &[u8]) -> String {
    format::digest(bytes)
}
