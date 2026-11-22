module str;

use option::Option;

// Text search over UTF-8 bytes.
//
// `find` uses the Two-Way algorithm: O(m) preprocessing with constant space
// and O(n + m) worst-case searching. A one-byte needle takes a linear scan.
// UTF-8 is self-synchronizing, so a byte match is also a scalar boundary in a
// valid string.
impl str {
  // Reports whether the string starts with `prefix`.
  public func starts_with(self: *str, prefix: *str) -> bool {
    bytes_equal_at(self.as_bytes(), 0, prefix.as_bytes())
  }

  // Reports whether the string ends with `suffix`.
  public func ends_with(self: *str, suffix: *str) -> bool {
    let haystack = self.as_bytes();
    let needle = suffix.as_bytes();
    if needle.len() > haystack.len() {
      return false;
    }
    bytes_equal_at(haystack, haystack.len() - needle.len(), needle)
  }

  // Reports whether `needle` occurs anywhere in the string. The empty needle is
  // contained by every string.
  public func contains(self: *str, needle: *str) -> bool {
    match self.find(needle) {
      Option::Some(_) => true,
      Option::None => false,
    }
  }

  // Returns the byte index of the first occurrence of `needle`, or
  // `Option::None` when it is absent. The empty needle occurs at index zero.
  public func find(self: *str, needle: *str) -> Option<usize> {
    find_bytes(self.as_bytes(), needle.as_bytes())
  }
}

// Finds `needle` at or after byte `start`.
func find_from(text: *str, needle: *str, start: usize) -> Option<usize> {
  if start > text.len() {
    return Option::None;
  }
  let rest = text.slice(start, text.len());
  match rest.find(needle) {
    Option::Some(index) => Option::Some(start + index),
    Option::None => Option::None,
  }
}

// Returns whether `count` bytes match starting at the given offsets.
func bytes_equal_range(
  left: *[u8],
  left_start: usize,
  right: *[u8],
  right_start: usize,
  count: usize,
) -> bool {
  if left_start > left.len() || count > left.len() - left_start {
    return false;
  }
  if right_start > right.len() || count > right.len() - right_start {
    return false;
  }
  let mut index: usize = 0;
  while index < count {
    if left[left_start + index] != right[right_start + index] {
      return false;
    }
    index += 1;
  }
  true
}

// Returns whether `needle` matches `haystack` starting at `offset`.
func bytes_equal_at(haystack: *[u8], offset: usize, needle: *[u8]) -> bool {
  bytes_equal_range(haystack, offset, needle, 0, needle.len())
}

func find_bytes(haystack: *[u8], needle: *[u8]) -> Option<usize> {
  let needle_len = needle.len();
  if needle_len == 0 {
    return Option::Some(0);
  }
  if needle_len > haystack.len() {
    return Option::None;
  }
  if needle_len == 1 {
    return find_single_byte(haystack, needle[0]);
  }
  find_two_way(haystack, needle)
}

func find_single_byte(haystack: *[u8], needle: u8) -> Option<usize> {
  let mut index: usize = 0;
  let length = haystack.len();
  while index < length {
    if haystack[index] == needle {
      return Option::Some(index);
    }
    index += 1;
  }
  Option::None
}
