module str;

// A non-allocating iterator over Unicode scalar values.
public struct Chars {
  source: *str,
  offset: usize,
}

// A non-allocating iterator over Unicode scalars and their byte offsets.
public struct CharIndices {
  source: *str,
  offset: usize,
}

impl str {
  // Returns an iterator over the string's Unicode scalars.
  public func chars(self: *str) -> Chars {
    Chars { source: self, offset: 0 }
  }

  // Returns an iterator over `(byte offset, Unicode scalar)` pairs.
  public func char_indices(self: *str) -> CharIndices {
    CharIndices { source: self, offset: 0 }
  }
}

impl Iterator<char> for Chars {
  func has_next(self: *Chars) -> bool {
    (*self).offset < (*self).source.len()
  }

  func next(self: *mut Chars) -> char {
    let source = (*self).source;
    let start = (*self).offset;
    let value = source.char_at(start);
    let end = start + value.len_utf8();
    (*self).offset = end;
    value
  }
}

impl Iterator<(usize, char)> for CharIndices {
  func has_next(self: *CharIndices) -> bool {
    (*self).offset < (*self).source.len()
  }

  func next(self: *mut CharIndices) -> (usize, char) {
    let source = (*self).source;
    let start = (*self).offset;
    let value = source.char_at(start);
    (*self).offset = start + value.len_utf8();
    (start, value)
  }
}

impl IntoIterator<Chars, char> for *str {
  func iter(self: *str) -> Chars {
    self.chars()
  }
}
