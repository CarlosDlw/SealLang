module str;

// A non-allocating iterator over the exact UTF-8 bytes of a string.
public struct Bytes {
  bytes: *[u8],
  index: usize,
}

impl str {
  // Returns an iterator over the string's bytes, in order.
  public func bytes(self: *str) -> Bytes {
    Bytes { bytes: self.as_bytes(), index: 0 }
  }
}

impl Iterator<u8> for Bytes {
  func has_next(self: *Bytes) -> bool {
    (*self).index < (*self).bytes.len()
  }

  func next(self: *mut Bytes) -> u8 {
    let value = (*self).bytes[(*self).index];
    (*self).index += 1;
    value
  }
}
