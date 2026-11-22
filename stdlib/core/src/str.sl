module str;

use option::Option;

// Borrowed Unicode text. A `str` value is reached through `*str`, a fat pointer
// carrying the data address and the exact byte length.
//
// All indices in this module are byte offsets, never character counts. A valid
// index is between zero and `len()` inclusive; an index at `len()` names the
// end of the string. `is_char_boundary` returns true there and false for every
// index past it. A range is end-exclusive, so `get(start, end)` and
// `slice(start, end)` cover `end - start` bytes.
//
// Every string reachable from safe code contains valid UTF-8. Methods that
// build a view either validate the range (`slice` panics, `get` returns
// `Option::None`) or validate the bytes (`from_utf8`). The unsafe constructors
// `from_utf8_unchecked` and `as_str_unchecked` require the caller to establish
// the UTF-8 contract.
//
// No function in this module allocates. A view borrows the storage of its
// source and cannot outlive it.
impl str {
  // Reports whether the string contains no bytes. This is O(1).
  public func is_empty(self: *str) -> bool {
    self.len() == 0
  }

  // Returns the substring covering `start..end`, or `Option::None` when either
  // index is past the end, `start > end`, or an index falls inside a multibyte
  // scalar.
  //
  // This is the total counterpart of `slice`: it never panics and accepts any
  // `usize` pair. The returned view borrows `self`.
  public func get(self: *str, start: usize, end: usize) -> Option<*str> {
    if start > end {
      return Option::None;
    }
    let len = self.len();
    if end > len {
      return Option::None;
    }
    if !self.is_char_boundary(start) || !self.is_char_boundary(end) {
      return Option::None;
    }
    Option::Some(self.slice(start, end))
  }

  // Splits the string at `index` and returns both halves.
  //
  // # Panics
  //
  // Panics when `index > self.len()` with `split_at index is out of bounds`,
  // and when `index` falls inside a multibyte scalar with
  // `split_at index is not a char boundary`.
  public func split_at(self: *str, index: usize) -> (*str, *str) {
    let end = self.len();
    if index > end {
      panic("split_at index is out of bounds");
    }
    if !self.is_char_boundary(index) {
      panic("split_at index is not a char boundary");
    }
    (self.slice(0, index), self.slice(index, end))
  }
}

// Builds a string view from bytes after validating the complete UTF-8
// encoding. Returns `Option::None` for malformed input and never allocates.
//
// The view borrows `bytes`; its loan keeps the byte storage alive for every
// use.
public func from_utf8(bytes: *[u8]) -> Option<*str> {
  if !is_valid_utf8(bytes) {
    return Option::None;
  }
  Option::Some(unsafe { bytes.as_str_unchecked() })
}

// Builds a string view from bytes without validating them.
//
// # Safety
//
// The caller must guarantee that the whole byte slice is valid UTF-8. The
// returned view borrows `bytes`.
public unsafe func from_utf8_unchecked(bytes: *[u8]) -> *str {
  unsafe { bytes.as_str_unchecked() }
}

// Reports whether `byte` is a UTF-8 continuation byte (`0b10xx_xxxx`).
func is_continuation(byte: u8) -> bool {
  byte >= 0x80 && byte <= 0xbf
}

// Validates the complete UTF-8 encoding of `bytes` in O(n) time.
//
// The validator rejects lone continuation bytes, truncated sequences, overlong
// encodings, surrogates, and scalar values above `U+10FFFF`. It reads only
// indices proven to be inside the slice.
func is_valid_utf8(bytes: *[u8]) -> bool {
  let len = bytes.len();
  let mut index: usize = 0;

  while index < len {
    let lead = bytes[index];

    if lead < 0x80 {
      index += 1;
    } else if lead >= 0xc2 && lead <= 0xdf {
      if len - index < 2 {
        return false;
      }
      if !is_continuation(bytes[index + 1]) {
        return false;
      }
      index += 2;
    } else if lead >= 0xe0 && lead <= 0xef {
      if len - index < 3 {
        return false;
      }
      let second = bytes[index + 1];
      let third = bytes[index + 2];
      let second_ok = if lead == 0xe0 {
        second >= 0xa0 && second <= 0xbf
      } else if lead == 0xed {
        second >= 0x80 && second <= 0x9f
      } else {
        is_continuation(second)
      };
      if !second_ok || !is_continuation(third) {
        return false;
      }
      index += 3;
    } else if lead >= 0xf0 && lead <= 0xf4 {
      if len - index < 4 {
        return false;
      }
      let second = bytes[index + 1];
      let third = bytes[index + 2];
      let fourth = bytes[index + 3];
      let second_ok = if lead == 0xf0 {
        second >= 0x90 && second <= 0xbf
      } else if lead == 0xf4 {
        second >= 0x80 && second <= 0x8f
      } else {
        is_continuation(second)
      };
      if !second_ok || !is_continuation(third) || !is_continuation(fourth) {
        return false;
      }
      index += 4;
    } else {
      return false;
    }
  }

  true
}
