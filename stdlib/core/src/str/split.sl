module str;

use char::is_whitespace;
use option::Option;

// A non-allocating iterator over the pieces produced by `str::split`.
public struct Split {
  source: *str,
  separator: *str,
  offset: usize,
  empty_separator: bool,
  started: bool,
  finished: bool,
}

// A non-allocating iterator over lines, using Rust-compatible newline rules.
public struct Lines {
  source: *str,
  offset: usize,
}

// A non-allocating iterator over Unicode-whitespace-separated words.
public struct SplitWhitespace {
  source: *str,
  offset: usize,
}

impl str {
  // Splits the string on every occurrence of `separator`.
  //
  // A non-empty separator preserves leading, trailing, and consecutive empty
  // pieces. An empty separator yields an empty piece before every scalar, plus
  // one at the end; an empty source therefore yields two empty pieces.
  public func split(self: *str, separator: *str) -> Split {
    Split {
      source: self,
      separator: separator,
      offset: 0,
      empty_separator: separator.len() == 0,
      started: false,
      finished: false,
    }
  }

  // Splits at the first occurrence of `separator`, returning the text before
  // and after it. The empty separator matches at the start, so the first piece
  // is empty and the second is the whole string.
  public func split_once(self: *str, separator: *str) -> Option<(*str, *str)> {
    if separator.len() == 0 {
      return Option::Some(("", self));
    }
    match self.find(separator) {
      Option::Some(index) => {
        let end = index + separator.len();
        Option::Some((self.slice(0, index), self.slice(end, self.len())))
      }
      Option::None => Option::None,
    }
  }

  // Returns an iterator over the string's lines.
  //
  // Lines are separated by `\n`. A `\r` immediately before `\n` is removed. A
  // trailing newline does not produce a final empty line, matching Rust.
  public func lines(self: *str) -> Lines {
    Lines { source: self, offset: 0 }
  }

  // Returns an iterator over words separated by Unicode `White_Space`.
  //
  // Leading, trailing, and repeated whitespace never produce empty items.
  public func split_whitespace(self: *str) -> SplitWhitespace {
    SplitWhitespace { source: self, offset: 0 }
  }
}

impl Iterator<*str> for Split {
  func has_next(self: *Split) -> bool {
    !(*self).finished
  }

  func next(self: *mut Split) -> *str {
    if (*self).finished {
      panic("split iterator is exhausted");
    }
    if !(*self).empty_separator {
      return split_nonempty(self);
    }
    if !(*self).started {
      (*self).started = true;
      return "";
    }
    let source = (*self).source;
    let offset = (*self).offset;
    if offset >= source.len() {
      (*self).finished = true;
      return "";
    }
    let value = source.char_at(offset);
    let end = offset + value.len_utf8();
    (*self).offset = end;
    view_between(source, offset, end)
  }
}

func split_nonempty(state: *mut Split) -> *str {
  let source = (*state).source;
  let offset = (*state).offset;
  let separator = (*state).separator;
  match find_from(source, separator, offset) {
    Option::Some(index) => split_found(state, source, index, separator.len()),
    Option::None => split_remainder(state, source),
  }
}

func split_found(state: *mut Split, source: *str, index: usize, width: usize) -> *str {
  let start = (*state).offset;
  let piece_end = index;
  (*state).offset = piece_end + width;
  view_between(source, start, piece_end)
}

func split_remainder(state: *mut Split, source: *str) -> *str {
  let start = (*state).offset;
  (*state).finished = true;
  (*state).offset = source.len();
  view_between(source, start, source.len())
}

impl Iterator<*str> for Lines {
  func has_next(self: *Lines) -> bool {
    (*self).offset < (*self).source.len()
  }

  func next(self: *mut Lines) -> *str {
    let source = (*self).source;
    let start = (*self).offset;
    if start >= source.len() {
      panic("lines iterator is exhausted");
    }
    match find_from(source, "\n", start) {
      Option::Some(index) => line_with_newline(self, source, start, index),
      Option::None => line_without_newline(self, source, start),
    }
  }
}

func line_with_newline(state: *mut Lines, source: *str, start: usize, newline: usize) -> *str {
  let mut end = newline;
  let bytes = source.as_bytes();
  if end > start && bytes[end - 1] == 13 {
    end -= 1;
  }
  (*state).offset = newline + 1;
  view_between(source, start, end)
}

func line_without_newline(state: *mut Lines, source: *str, start: usize) -> *str {
  (*state).offset = source.len();
  view_between(source, start, source.len())
}

impl Iterator<*str> for SplitWhitespace {
  func has_next(self: *SplitWhitespace) -> bool {
    skip_whitespace((*self).source, (*self).offset) < (*self).source.len()
  }

  func next(self: *mut SplitWhitespace) -> *str {
    let source = (*self).source;
    let start = skip_whitespace(source, (*self).offset);
    if start >= source.len() {
      (*self).offset = source.len();
      return "";
    }
    let mut end = start;
    while end < source.len() {
      let value = source.char_at(end);
      if is_whitespace(value) {
        break;
      }
      end += value.len_utf8();
    }
    (*self).offset = end;
    view_between(source, start, end)
  }
}

// Returns the offset after all leading Unicode whitespace at or after `start`.
func skip_whitespace(text: *str, start: usize) -> usize {
  let mut offset = start;
  while offset < text.len() {
    let value = text.char_at(offset);
    if !is_whitespace(value) {
      break;
    }
    offset += value.len_utf8();
  }
  offset
}
