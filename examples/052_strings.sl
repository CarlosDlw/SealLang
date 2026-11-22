// Strings are borrowed UTF-8 views. This program uses the primitive `str`
// operations directly so it runs without the standard library: byte offsets,
// checked substrings, scalar decoding, byte views, a linear search, splitting,
// Unicode whitespace trimming, and a validated UTF-8 constructor.
//
// The standard library builds the same operations on these primitives; see
// `examples/stdlib` for the high-level surface. Every unsafe operation here is
// confined to the constructor that runs after full validation.
//
// The documented result is 0. Any wrong offset, byte, scalar, or construction
// returns a non-zero status.

func find_bytes(haystack: *[u8], needle: *[u8], start: usize) -> usize {
  let haystack_len = haystack.len();
  let needle_len = needle.len();
  if start > haystack_len {
    return haystack_len;
  }
  if needle_len == 0 {
    return start;
  }

  let mut index = start;
  while needle_len <= haystack_len - index {
    let mut offset: usize = 0;
    let mut matched = true;
    while offset < needle_len {
      if haystack[index + offset] != needle[offset] {
        matched = false;
        break;
      }
      offset += 1;
    }
    if matched {
      return index;
    }
    index += 1;
  }
  haystack_len
}

func count_pieces(text: *str, separator: *str) -> usize {
  let bytes = text.as_bytes();
  let pattern = separator.as_bytes();
  if pattern.len() == 0 {
    return 0;
  }

  let mut count: usize = 1;
  let mut start: usize = 0;
  while start <= bytes.len() {
    let found = find_bytes(bytes, pattern, start);
    if found == bytes.len() {
      return count;
    }
    count += 1;
    start = found + pattern.len();
  }
  count
}

func is_whitespace(value: char) -> bool {
  let code = value.to_u32();
  (code >= 0x9 && code <= 0xd)
    || code == 0x20
    || code == 0x85
    || code == 0xa0
    || code == 0x1680
    || (code >= 0x2000 && code <= 0x200a)
    || code == 0x2028
    || code == 0x2029
    || code == 0x202f
    || code == 0x205f
    || code == 0x3000
}

func trim(text: *str) -> *str {
  let mut start: usize = 0;
  while start < text.len() {
    let value = text.char_at(start);
    if !is_whitespace(value) {
      break;
    }
    start += value.len_utf8();
  }

  let mut end = text.len();
  while end > start {
    let mut previous = end - 1;
    while previous > start && !text.is_char_boundary(previous) {
      previous -= 1;
    }
    if !is_whitespace(text.char_at(previous)) {
      break;
    }
    end = previous;
  }
  text.slice(start, end)
}

func is_continuation(byte: u8) -> bool {
  byte >= 0x80 && byte <= 0xbf
}

func is_valid_utf8(bytes: *[u8]) -> bool {
  let len = bytes.len();
  let mut index: usize = 0;

  while index < len {
    let lead = bytes[index];
    if lead < 0x80 {
      index += 1;
    } else if lead >= 0xc2 && lead <= 0xdf {
      if len - index < 2 || !is_continuation(bytes[index + 1]) {
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

// Validates the complete byte slice before constructing the view. The view
// borrows `bytes`; its loan keeps the storage alive for every use.
func checked_str(bytes: *[u8]) -> *str {
  if !is_valid_utf8(bytes) {
    panic("checked_str received invalid UTF-8");
  }
  // SAFETY: the validation above established the UTF-8 invariant for exactly
  // the bytes and length carried by this slice pointer.
  unsafe { bytes.as_str_unchecked() }
}

func main() -> i32 {
  let text: *str = "a🦀é";
  if text.len() != 7 || text.len() == 0 {
    return 1;
  }
  if !text.is_char_boundary(0)
    || !text.is_char_boundary(1)
    || text.is_char_boundary(2)
    || !text.is_char_boundary(5)
    || !text.is_char_boundary(7)
  {
    return 2;
  }
  if text.char_at(0) != 'a' || text.char_at(1) != '🦀' || text.char_at(5) != 'é' {
    return 3;
  }
  if text.slice(1, 5) != "🦀" || text.slice(5, 7) != "é" {
    return 4;
  }

  let bytes = text.as_bytes();
  let mut byte_count: usize = 0;
  let mut byte_total: usize = 0;
  while byte_count < bytes.len() {
    byte_total += bytes[byte_count] as usize;
    byte_count += 1;
  }
  if byte_count != 7 || byte_total == 0 {
    return 5;
  }

  let mut offset: usize = 0;
  let mut scalar_count: usize = 0;
  while offset < text.len() {
    let value = text.char_at(offset);
    if value.len_utf8() == 0 {
      return 6;
    }
    offset += value.len_utf8();
    scalar_count += 1;
  }
  if scalar_count != 3 || offset != text.len() {
    return 7;
  }

  if find_bytes(bytes, "🦀".as_bytes(), 0) != 1 {
    return 8;
  }
  if find_bytes("banana".as_bytes(), "ana".as_bytes(), 0) != 1 {
    return 9;
  }
  if find_bytes(bytes, "zz".as_bytes(), 0) != bytes.len() {
    return 10;
  }

  if count_pieces("a,,b,", ",") != 4 || count_pieces("abc", ",") != 1 {
    return 11;
  }
  if trim("  seal \t") != "seal" || trim("\u{2003}seal\u{3000}") != "seal" {
    return 12;
  }
  if trim("\u{200b}") != "\u{200b}" || trim("") != "" {
    return 13;
  }

  if !is_valid_utf8(bytes) {
    return 14;
  }
  let rebuilt = checked_str(bytes);
  if rebuilt != text {
    return 15;
  }
  let invalid: [u8; 2] = [0xc0u8, 0x80u8];
  if is_valid_utf8(&invalid[..]) {
    return 16;
  }

  0
}
