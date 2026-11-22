module str;

// Borrowed Unicode text. A `str` value is reached through `*str`, a fat pointer
// carrying the data address and the exact byte length. All indices in this
// module are byte offsets, and every safe view starts and ends on a UTF-8
// scalar boundary.
impl str {
  // Reports whether the string contains no bytes. Foundation methods such as
  // `len` are provided by the compiler catalog because they read the fat
  // pointer metadata directly.
  public func is_empty(self: *str) -> bool {
    self.len() == 0
  }
}
