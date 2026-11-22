module str;

impl str {
  // Compares two strings while ignoring ASCII letter case.
  //
  // Only `A..Z` and `a..z` are folded. Non-ASCII bytes are compared exactly,
  // so this is not full Unicode case folding. No allocation is performed.
  public func eq_ignore_ascii_case(self: *str, other: *str) -> bool {
    let left = self.as_bytes();
    let right = other.as_bytes();
    if left.len() != right.len() {
      return false;
    }
    let mut index: usize = 0;
    while index < left.len() {
      if ascii_lower(left[index]) != ascii_lower(right[index]) {
        return false;
      }
      index += 1;
    }
    true
  }
}

func ascii_lower(byte: u8) -> u8 {
  if byte >= 65 && byte <= 90 {
    byte + 32
  } else {
    byte
  }
}
