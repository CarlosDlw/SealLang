//! Line indexing for source text.
//!
//! This is the only place in the compiler that counts line breaks. Everything
//! else asks a [`LineIndex`]. Column numbers are one-based and counted in
//! Unicode scalar values; byte columns are available separately for tooling that
//! needs them.
//!
//! Line endings follow the language rule: a line ends at `\n`. A `\r`
//! immediately before `\n` belongs to the line ending, not to the line text.
//! Other carriage returns are ordinary characters and are the lexer's problem.

use std::fmt;
use std::ops::Range;

use crate::error::SpanError;

/// A one-based human-readable position inside a file.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LineCol {
    /// One-based line number.
    pub line: u32,
    /// One-based column number, counted in Unicode scalar values.
    pub column: u32,
    /// Zero-based byte column inside the line.
    pub byte_column: u32,
}

impl LineCol {
    /// Returns the one-based UTF-16 column, as required by the Language Server
    /// Protocol.
    pub fn utf16_column(self, text: &str, line_start: u32) -> u32 {
        let end = (line_start + self.byte_column) as usize;
        let start = line_start as usize;
        let prefix = text.get(start..end).unwrap_or("");
        prefix.encode_utf16().count() as u32 + 1
    }
}

impl fmt::Debug for LineCol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

impl fmt::Display for LineCol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// Byte offsets of the start of every line in a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineIndex {
    /// Always starts with `0`; one entry per line, sorted ascending.
    starts: Box<[u32]>,
}

impl LineIndex {
    /// Builds the index for `text`.
    ///
    /// # Panics
    ///
    /// Panics if `text` is longer than `u32::MAX` bytes. [`crate::SourceMap`]
    /// rejects such files before building an index.
    pub fn new(text: &str) -> Self {
        let mut starts = Vec::with_capacity(text.len() / 32 + 1);
        starts.push(0u32);
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                let next = u32::try_from(offset + 1)
                    .expect("source files longer than u32::MAX bytes are rejected earlier");
                starts.push(next);
            }
        }
        Self {
            starts: starts.into_boxed_slice(),
        }
    }

    /// Returns the number of lines. A trailing newline produces a final empty
    /// line, mirroring how editors display the file.
    #[inline]
    pub fn line_count(&self) -> u32 {
        self.starts.len() as u32
    }

    /// Returns the byte offset where `line` (zero-based) starts.
    #[inline]
    pub fn line_start(&self, line: u32) -> Option<u32> {
        self.starts.get(line as usize).copied()
    }

    /// Returns the zero-based line containing `offset`.
    ///
    /// # Panics
    ///
    /// Panics if `offset` is greater than `u32::MAX`; positions come from spans
    /// and are always representable.
    #[inline]
    pub fn line_of(&self, offset: u32) -> u32 {
        let after = self.starts.partition_point(|&start| start <= offset);
        (after.saturating_sub(1)) as u32
    }

    /// Converts a byte offset into a one-based line and column.
    pub fn line_col(&self, text: &str, offset: u32) -> Result<LineCol, SpanError> {
        let offset_usize = offset as usize;
        let length = text.len() as u32;
        if offset > length {
            return Err(SpanError::OutOfBounds {
                end: offset,
                len: length,
            });
        }
        if !text.is_char_boundary(offset_usize) {
            return Err(SpanError::NotCharBoundary { offset });
        }
        let line = self.line_of(offset);
        let line_start = self.line_start(line).expect("line exists");
        let byte_column = offset - line_start;
        let prefix = &text[line_start as usize..offset_usize];
        Ok(LineCol {
            line: line + 1,
            column: prefix.chars().count() as u32 + 1,
            byte_column,
        })
    }

    /// Returns the byte range of `line` (zero-based) without its line ending.
    pub fn line_range(&self, text: &str, line: u32) -> Option<Range<u32>> {
        let start = self.line_start(line)?;
        let mut end = self.line_start(line + 1).unwrap_or(text.len() as u32);
        if end > start && text.as_bytes().get(end as usize - 1) == Some(&b'\n') {
            end -= 1;
        }
        if end > start && text.as_bytes().get(end as usize - 1) == Some(&b'\r') {
            end -= 1;
        }
        Some(start..end)
    }

    /// Returns the text of `line` (zero-based), without its line ending.
    pub fn line_text<'a>(&self, text: &'a str, line: u32) -> Option<&'a str> {
        let range = self.line_range(text, line)?;
        Some(&text[range.start as usize..range.end as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::LineIndex;
    use crate::error::SpanError;

    fn index(text: &str) -> LineIndex {
        LineIndex::new(text)
    }

    #[test]
    fn empty_text_has_one_empty_line() {
        let text = "";
        let index = index(text);
        assert_eq!(index.line_count(), 1);
        assert_eq!(index.line_range(text, 0), Some(0..0));
        assert_eq!(index.line_text(text, 0), Some(""));
        assert_eq!(index.line_start(1), None);
    }

    #[test]
    fn lf_line_endings() {
        let text = "a\nbb\nccc";
        let index = index(text);
        assert_eq!(index.line_count(), 3);
        assert_eq!(index.line_text(text, 0), Some("a"));
        assert_eq!(index.line_text(text, 1), Some("bb"));
        assert_eq!(index.line_text(text, 2), Some("ccc"));
    }

    #[test]
    fn trailing_newline_creates_an_empty_line() {
        let text = "a\n";
        let index = index(text);
        assert_eq!(index.line_count(), 2);
        assert_eq!(index.line_text(text, 1), Some(""));
    }

    #[test]
    fn crlf_is_a_single_line_ending() {
        let text = "a\r\nb\r\n";
        let index = index(text);
        assert_eq!(index.line_count(), 3);
        assert_eq!(index.line_text(text, 0), Some("a"));
        assert_eq!(index.line_text(text, 1), Some("b"));
        assert_eq!(index.line_text(text, 2), Some(""));
    }

    #[test]
    fn lone_carriage_return_stays_in_the_line() {
        let text = "a\rb";
        let index = index(text);
        assert_eq!(index.line_count(), 1);
        assert_eq!(index.line_text(text, 0), Some("a\rb"));
    }

    #[test]
    fn columns_count_characters_not_bytes() {
        let text = "ab\né日x";
        let index = index(text);
        let start_of_second_line = 3u32;
        let e_acute = index
            .line_col(text, start_of_second_line)
            .expect("valid offset");
        assert_eq!((e_acute.line, e_acute.column), (2, 1));
        let cjk = index
            .line_col(text, start_of_second_line + 2)
            .expect("valid offset");
        assert_eq!((cjk.line, cjk.column, cjk.byte_column), (2, 2, 2));
        let x = index
            .line_col(text, start_of_second_line + 5)
            .expect("valid offset");
        assert_eq!((x.line, x.column, x.byte_column), (2, 3, 5));
    }

    #[test]
    fn utf16_columns_account_for_surrogate_pairs() {
        let text = "🔥x";
        let index = index(text);
        let x = index.line_col(text, 4).expect("valid offset");
        assert_eq!(x.column, 2);
        assert_eq!(x.utf16_column(text, 0), 3);
    }

    #[test]
    fn offsets_must_be_char_boundaries() {
        let text = "é";
        let index = index(text);
        assert_eq!(
            index.line_col(text, 1),
            Err(SpanError::NotCharBoundary { offset: 1 })
        );
    }

    #[test]
    fn offset_past_end_is_rejected() {
        let text = "a";
        let index = index(text);
        assert_eq!(
            index.line_col(text, 2),
            Err(SpanError::OutOfBounds { end: 2, len: 1 })
        );
    }

    #[test]
    fn eof_position_is_valid() {
        let text = "a\nb";
        let index = index(text);
        let eof = index.line_col(text, 3).expect("valid offset");
        assert_eq!((eof.line, eof.column), (2, 2));
    }

    #[test]
    fn line_of_boundaries() {
        let text = "a\nb\nc";
        let index = index(text);
        assert_eq!(index.line_of(0), 0);
        assert_eq!(index.line_of(1), 0);
        assert_eq!(index.line_of(2), 1);
        assert_eq!(index.line_of(4), 2);
    }
}
