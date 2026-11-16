//! Registered source files.

use std::ops::Range;
use std::sync::OnceLock;

use seal_hash::{Fingerprint, stable_hash_of};

use crate::error::SpanError;
use crate::file_name::FileName;
use crate::line_index::{LineCol, LineIndex};
use crate::span::{BytePos, Span};

/// A source file registered in a [`crate::SourceMap`].
///
/// The text is stored after BOM removal, so all byte offsets are relative to the
/// file as the compiler sees it, not to the bytes on disk.
pub struct SourceFile {
    name: FileName,
    text: Box<str>,
    start: BytePos,
    end: BytePos,
    has_bom: bool,
    content_hash: Fingerprint,
    lines: OnceLock<LineIndex>,
}

impl SourceFile {
    pub(crate) fn new(name: FileName, text: String, start: BytePos, has_bom: bool) -> Self {
        let content_hash = stable_hash_of(text.as_str());
        let end = BytePos::from_u32(start.to_u32() + text.len() as u32);
        Self {
            name,
            text: text.into_boxed_str(),
            start,
            end,
            has_bom,
            content_hash,
            lines: OnceLock::new(),
        }
    }

    /// Returns the file name.
    #[inline]
    pub fn name(&self) -> &FileName {
        &self.name
    }

    /// Returns the file text, without a leading BOM.
    #[inline]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the global position where this file starts.
    #[inline]
    pub fn start(&self) -> BytePos {
        self.start
    }

    /// Returns the global position where this file ends (exclusive).
    #[inline]
    pub fn end(&self) -> BytePos {
        self.end
    }

    /// Returns the file length in bytes.
    #[inline]
    pub fn len(&self) -> u32 {
        self.text.len() as u32
    }

    /// Returns `true` for an empty file.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Returns `true` if the file on disk started with a UTF-8 BOM, which was
    /// removed from the stored text.
    #[inline]
    pub fn has_bom(&self) -> bool {
        self.has_bom
    }

    /// Returns a stable hash of the file contents.
    #[inline]
    pub fn content_hash(&self) -> Fingerprint {
        self.content_hash
    }

    /// Returns the lazily built line index.
    pub fn lines(&self) -> &LineIndex {
        self.lines.get_or_init(|| LineIndex::new(&self.text))
    }

    /// Returns `true` if the global position is inside this file.
    #[inline]
    pub fn contains(&self, position: BytePos) -> bool {
        self.start <= position && position <= self.end
    }

    /// Returns `true` if the span is fully inside this file.
    #[inline]
    pub fn contains_span(&self, span: Span) -> bool {
        self.start <= span.lo() && span.hi() <= self.end
    }

    /// Converts a global position into a file-local byte offset.
    #[inline]
    pub fn local_offset(&self, position: BytePos) -> Option<u32> {
        if self.start <= position && position <= self.end {
            Some(position.to_u32() - self.start.to_u32())
        } else {
            None
        }
    }

    /// Converts a file-local byte offset into a global span.
    pub fn span(&self, range: Range<u32>) -> Result<Span, SpanError> {
        self.validate_local_range(&range)?;
        let start = self.start.to_u32();
        Ok(Span::new(
            BytePos::from_u32(start + range.start),
            BytePos::from_u32(start + range.end),
        ))
    }

    /// Returns the span covering the whole file.
    #[inline]
    pub fn full_span(&self) -> Span {
        Span::new(self.start, self.end)
    }

    /// Returns the empty span at the file start.
    #[inline]
    pub fn empty_span(&self) -> Span {
        Span::new(self.start, self.start)
    }

    /// Returns the text covered by a global span.
    pub fn snippet(&self, span: Span) -> Result<&str, SpanError> {
        let lo = self
            .local_offset(span.lo())
            .ok_or(SpanError::SpanCrossesFiles)?;
        let hi = self
            .local_offset(span.hi())
            .ok_or(SpanError::SpanCrossesFiles)?;
        self.validate_local_range(&(lo..hi))?;
        Ok(&self.text[lo as usize..hi as usize])
    }

    /// Returns the one-based position of a global position.
    pub fn line_col(&self, position: BytePos) -> Result<LineCol, SpanError> {
        let local = self
            .local_offset(position)
            .ok_or(SpanError::SpanCrossesFiles)?;
        self.lines().line_col(&self.text, local)
    }

    /// Returns the text of a zero-based line, without its line ending.
    pub fn line_text(&self, line: u32) -> Option<&str> {
        self.lines().line_text(&self.text, line)
    }

    /// Returns the byte range of a zero-based line, without its line ending.
    pub fn line_range(&self, line: u32) -> Option<Range<u32>> {
        self.lines().line_range(&self.text, line)
    }

    /// Returns the zero-based line containing a global position.
    pub fn line_of(&self, position: BytePos) -> Option<u32> {
        let local = self.local_offset(position)?;
        Some(self.lines().line_of(local))
    }

    fn validate_local_range(&self, range: &Range<u32>) -> Result<(), SpanError> {
        if range.start > range.end {
            return Err(SpanError::InvalidRange {
                start: range.start,
                end: range.end,
            });
        }
        if range.end > self.len() {
            return Err(SpanError::OutOfBounds {
                end: range.end,
                len: self.len(),
            });
        }
        if !self.text.is_char_boundary(range.start as usize) {
            return Err(SpanError::NotCharBoundary {
                offset: range.start,
            });
        }
        if !self.text.is_char_boundary(range.end as usize) {
            return Err(SpanError::NotCharBoundary { offset: range.end });
        }
        Ok(())
    }
}

impl std::fmt::Debug for SourceFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SourceFile")
            .field("name", &self.name)
            .field("len", &self.len())
            .field("start", &self.start)
            .field("has_bom", &self.has_bom)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::SourceFile;
    use crate::error::SpanError;
    use crate::file_name::FileName;
    use crate::span::BytePos;

    fn file(text: &str) -> SourceFile {
        SourceFile::new(
            FileName::virtual_name("test"),
            text.to_owned(),
            BytePos::from_u32(100),
            false,
        )
    }

    #[test]
    fn local_offsets_and_spans() {
        let file = file("hello");
        assert_eq!(file.len(), 5);
        assert_eq!(file.start(), BytePos::from_u32(100));
        assert_eq!(file.end(), BytePos::from_u32(105));
        let span = file.span(1..3).expect("valid range");
        assert_eq!(span.lo(), BytePos::from_u32(101));
        assert_eq!(span.hi(), BytePos::from_u32(103));
        assert_eq!(file.snippet(span), Ok("el"));
        assert_eq!(file.local_offset(BytePos::from_u32(104)), Some(4));
    }

    #[test]
    fn invalid_ranges_are_rejected() {
        let file = file("é");
        assert_eq!(
            file.span(1..2),
            Err(SpanError::NotCharBoundary { offset: 1 })
        );
        assert_eq!(
            file.span(0..3),
            Err(SpanError::OutOfBounds { end: 3, len: 2 })
        );
        let (start, end) = (2, 1);
        assert_eq!(
            file.span(start..end),
            Err(SpanError::InvalidRange { start: 2, end: 1 })
        );
    }

    #[test]
    fn snippet_outside_file_is_rejected() {
        let file = file("abc");
        let foreign = crate::SourceMap::new()
            .add_file(FileName::virtual_name("other"), "xyz".to_owned())
            .expect("file registers");
        let _ = foreign;
        let span = crate::span::Span::new(BytePos::from_u32(0), BytePos::from_u32(1));
        assert_eq!(file.snippet(span), Err(SpanError::SpanCrossesFiles));
    }

    #[test]
    fn content_hash_changes_with_content() {
        let a = file("abc");
        let b = file("abd");
        assert_ne!(a.content_hash(), b.content_hash());
        assert_eq!(a.content_hash(), file("abc").content_hash());
    }

    #[test]
    fn line_helpers_delegate_to_line_index() {
        let file = file("a\nbc");
        assert_eq!(file.line_text(0), Some("a"));
        assert_eq!(file.line_text(1), Some("bc"));
        assert_eq!(file.line_of(BytePos::from_u32(102)), Some(1));
        assert_eq!(file.line_range(1), Some(2..4));
    }
}
