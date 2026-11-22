module str;

use char::is_whitespace;

impl str {
  // Removes leading Unicode whitespace.
  public func trim_start(self: *str) -> *str {
    let mut offset: usize = 0;
    while offset < self.len() {
      let value = self.char_at(offset);
      if !is_whitespace(value) {
        break;
      }
      offset += value.len_utf8();
    }
    view_between(self, offset, self.len())
  }

  // Removes trailing Unicode whitespace.
  public func trim_end(self: *str) -> *str {
    let mut offset: usize = 0;
    let mut end: usize = 0;
    while offset < self.len() {
      let value = self.char_at(offset);
      let width = value.len_utf8();
      if !is_whitespace(value) {
        end = offset + width;
      }
      offset += width;
    }
    view_between(self, 0, end)
  }

  // Removes leading and trailing Unicode whitespace.
  public func trim(self: *str) -> *str {
    self.trim_start().trim_end()
  }
}
