//! Compact source spans.
//!
//! A [`Span`] is a half-open byte range in the *global* position space of a
//! [`crate::SourceMap`]. Global positions let spans stay 8 bytes, which matters
//! because every AST node stores one. The position space is an implementation
//! detail: spans are only created by [`crate::SourceMap`], which guarantees that
//! a span never crosses a file boundary, and are translated back to a file with
//! [`crate::SourceMap::file_of`].

use std::fmt;

/// A byte position in the global source-map position space.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BytePos(u32);

impl BytePos {
    /// Creates a position from a raw global offset.
    #[inline]
    pub const fn from_u32(value: u32) -> Self {
        Self(value)
    }

    /// Returns the raw global offset.
    #[inline]
    pub const fn to_u32(self) -> u32 {
        self.0
    }

    /// Returns the raw global offset as `usize`.
    #[inline]
    pub const fn to_usize(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Debug for BytePos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BytePos({})", self.0)
    }
}

/// A half-open byte range inside one source file.
///
/// Fields are private because only a source map can validate a span. Two spans
/// are ordered lexicographically by start, then end, which gives deterministic
/// diagnostic ordering.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Span {
    lo: BytePos,
    hi: BytePos,
}

impl Span {
    /// Creates a span. Crate-internal: `SourceMap` validates bounds and UTF-8
    /// boundaries before calling this.
    #[inline]
    pub(crate) const fn new(lo: BytePos, hi: BytePos) -> Self {
        Self { lo, hi }
    }

    /// Returns the start position.
    #[inline]
    pub const fn lo(self) -> BytePos {
        self.lo
    }

    /// Returns the exclusive end position.
    #[inline]
    pub const fn hi(self) -> BytePos {
        self.hi
    }

    /// Returns the length in bytes.
    #[inline]
    pub const fn len(self) -> u32 {
        self.hi.0 - self.lo.0
    }

    /// Returns `true` if the span covers no bytes.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.lo.0 == self.hi.0
    }

    /// Returns `true` if `position` lies inside the span.
    #[inline]
    pub const fn contains(self, position: BytePos) -> bool {
        self.lo.0 <= position.0 && position.0 < self.hi.0
    }

    /// Returns `true` if `other` is fully contained in this span.
    #[inline]
    pub const fn contains_span(self, other: Span) -> bool {
        self.lo.0 <= other.lo.0 && other.hi.0 <= self.hi.0
    }
}

impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.lo.0, self.hi.0)
    }
}

#[cfg(test)]
mod tests {
    use super::{BytePos, Span};

    #[test]
    fn span_predicates() {
        let span = Span::new(BytePos::from_u32(10), BytePos::from_u32(20));
        assert_eq!(span.len(), 10);
        assert!(!span.is_empty());
        assert!(span.contains(BytePos::from_u32(10)));
        assert!(span.contains(BytePos::from_u32(19)));
        assert!(!span.contains(BytePos::from_u32(20)));
        assert!(span.contains_span(Span::new(BytePos::from_u32(12), BytePos::from_u32(15))));
        assert!(!span.contains_span(Span::new(BytePos::from_u32(12), BytePos::from_u32(21))));
    }

    #[test]
    fn empty_span_contains_nothing() {
        let span = Span::new(BytePos::from_u32(5), BytePos::from_u32(5));
        assert!(span.is_empty());
        assert_eq!(span.len(), 0);
        assert!(!span.contains(BytePos::from_u32(5)));
    }

    #[test]
    fn span_is_eight_bytes() {
        assert_eq!(std::mem::size_of::<Span>(), 8);
    }
}
