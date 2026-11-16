//! Source files, positions, spans, and snippets.
//!
//! This crate is the single source of truth for "where" in the compiler:
//!
//! - [`SourceMap`] owns every registered file and the global position space.
//! - [`Span`] is an 8-byte half-open byte range inside exactly one file.
//! - [`LineIndex`] is the only code that counts line breaks and converts byte
//!   offsets to one-based line/column positions.
//! - [`FileName`] / [`PathRemapper`] handle reproducible path display.
//!
//! Invariants enforced here:
//!
//! - A span is only created through [`SourceMap::span`], which validates bounds,
//!   ordering, and UTF-8 character boundaries.
//! - A span never crosses a file boundary; [`SourceMap::file_of`] rejects ranges
//!   that cover the inter-file gap.
//! - Columns are counted in Unicode scalar values; UTF-16 columns are available
//!   separately for the Language Server Protocol.
//! - A leading UTF-8 BOM is removed once at registration and recorded.

#![warn(missing_docs)]

mod error;
mod file;
mod file_name;
mod line_index;
mod remap;
mod source_map;
mod span;

pub use error::{RemapError, SourceMapError, SpanError};
pub use file::SourceFile;
pub use file_name::FileName;
pub use line_index::{LineCol, LineIndex};
pub use remap::PathRemapper;
pub use source_map::{FileId, MAX_FILE_SIZE, Snippet, SourceMap};
pub use span::{BytePos, Span};
