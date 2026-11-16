//! Typed errors produced by source tracking.

use std::fmt;

/// Failure to construct or interpret a [`crate::Span`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpanError {
    /// The file id does not belong to this source map.
    UnknownFile,
    /// `start > end`.
    InvalidRange {
        /// Requested start offset.
        start: u32,
        /// Requested end offset.
        end: u32,
    },
    /// The range extends past the end of the file.
    OutOfBounds {
        /// Requested end offset.
        end: u32,
        /// File length in bytes.
        len: u32,
    },
    /// An offset is not on a UTF-8 character boundary.
    NotCharBoundary {
        /// The offending byte offset.
        offset: u32,
    },
    /// The span does not lie inside a single file.
    SpanCrossesFiles,
}

impl fmt::Display for SpanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownFile => f.write_str("unknown source file"),
            Self::InvalidRange { start, end } => {
                write!(f, "invalid span range: start {start} is after end {end}")
            }
            Self::OutOfBounds { end, len } => {
                write!(f, "span end {end} is beyond the file length {len}")
            }
            Self::NotCharBoundary { offset } => {
                write!(
                    f,
                    "byte offset {offset} is not on a UTF-8 character boundary"
                )
            }
            Self::SpanCrossesFiles => f.write_str("span crosses a file boundary"),
        }
    }
}

impl std::error::Error for SpanError {}

/// Failure to register a source file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceMapError {
    /// The file is too large for 32-bit source positions.
    FileTooLarge {
        /// Length in bytes.
        len: u64,
        /// Maximum accepted length.
        max: u32,
    },
    /// Adding the file would overflow the global position space.
    PositionOverflow {
        /// Starting position of the file.
        start: u32,
        /// Length of the file.
        len: u32,
    },
    /// More than `u32::MAX - 1` files were registered in one compilation.
    TooManyFiles,
}

impl fmt::Display for SourceMapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileTooLarge { len, max } => {
                write!(
                    f,
                    "source file is {len} bytes, exceeding the {max}-byte limit"
                )
            }
            Self::PositionOverflow { start, len } => write!(
                f,
                "source positions overflow: file starts at {start} with length {len}"
            ),
            Self::TooManyFiles => f.write_str("too many source files registered"),
        }
    }
}

impl std::error::Error for SourceMapError {}

/// Failure to parse a `--remap-path-prefix` specification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemapError {
    /// The specification is missing the `=`.
    MissingSeparator {
        /// The offending specification.
        spec: String,
    },
    /// The `from` prefix is empty.
    EmptyPrefix {
        /// The offending specification.
        spec: String,
    },
}

impl fmt::Display for RemapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSeparator { spec } => {
                write!(f, "invalid path remapping `{spec}`: expected `from=to`")
            }
            Self::EmptyPrefix { spec } => {
                write!(
                    f,
                    "invalid path remapping `{spec}`: the source prefix is empty"
                )
            }
        }
    }
}

impl std::error::Error for RemapError {}
