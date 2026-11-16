//! A cursor over the bytes of a source file.
//!
//! The cursor never slices at a non-boundary: character access goes through
//! `char`, and byte access is only used for ASCII decisions. `bump_byte` asserts
//! ASCII in debug builds, which catches a byte-oriented branch that forgot to
//! check the byte first.

use std::ops::Range;

/// A position in a `&str` that always stands on a UTF-8 boundary.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cursor<'a> {
    text: &'a str,
    position: usize,
}

impl<'a> Cursor<'a> {
    /// Creates a cursor at the start of `text`.
    pub(crate) fn new(text: &'a str) -> Self {
        Self { text, position: 0 }
    }

    /// Returns the current byte position.
    #[inline]
    pub(crate) fn position(self) -> usize {
        self.position
    }

    /// Returns the remaining text.
    #[inline]
    pub(crate) fn rest(self) -> &'a str {
        &self.text[self.position..]
    }

    /// Returns `true` at the end of the input.
    #[inline]
    pub(crate) fn is_eof(self) -> bool {
        self.position == self.text.len()
    }

    /// Returns the next character.
    #[inline]
    pub(crate) fn first(self) -> Option<char> {
        self.rest().chars().next()
    }

    /// Returns the next byte, if the input is not exhausted.
    #[inline]
    pub(crate) fn first_byte(self) -> Option<u8> {
        self.text.as_bytes().get(self.position).copied()
    }

    /// Returns the byte after the next one.
    #[inline]
    pub(crate) fn second_byte(self) -> Option<u8> {
        self.text.as_bytes().get(self.position + 1).copied()
    }

    /// Returns the byte at `offset` bytes from the cursor, if present.
    #[inline]
    pub(crate) fn byte_at(self, offset: usize) -> Option<u8> {
        self.text
            .as_bytes()
            .get(self.position.checked_add(offset)?)
            .copied()
    }

    /// Returns the `index`-th character from the cursor (zero-based).
    pub(crate) fn nth_char(self, index: usize) -> Option<char> {
        self.rest().chars().nth(index)
    }

    /// Advances over one character.
    pub(crate) fn bump_char(&mut self) -> Option<char> {
        let character = self.first()?;
        self.position += character.len_utf8();
        Some(character)
    }

    /// Advances over one ASCII byte.
    pub(crate) fn bump_byte(&mut self) -> Option<u8> {
        let byte = self.first_byte()?;
        debug_assert!(
            byte.is_ascii(),
            "bump_byte called on a non-ASCII byte at {}",
            self.position
        );
        self.position += 1;
        Some(byte)
    }

    /// Advances over `byte` and reports whether it matched.
    pub(crate) fn eat_byte(&mut self, byte: u8) -> bool {
        if self.first_byte() == Some(byte) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    /// Returns the text in `range`.
    ///
    /// # Panics
    ///
    /// Panics if the range is out of bounds or not on character boundaries. All
    /// callers pass ranges produced by the cursor itself.
    pub(crate) fn slice(self, range: Range<usize>) -> &'a str {
        &self.text[range]
    }
}
