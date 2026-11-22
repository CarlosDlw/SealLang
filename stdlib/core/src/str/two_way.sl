module str;

use option::Option;

// Two-Way search over UTF-8 bytes.
//
// The needle is split at a critical position into `u` and `v`. The search
// first matches `v` from left to right and then `u` from right to left, using
// the period of the chosen maximal or minimal suffix to skip candidates. The
// preprocessing is O(m) with constant space and the search is O(n + m) in the
// worst case.

// Returns the starting position and period of the minimal or maximal suffix.
func suffix(needle: *[u8], maximal: bool) -> (usize, usize) {
  let mut position: usize = 0;
  let mut period: usize = 1;
  let mut candidate: usize = 1;
  let mut offset: usize = 0;

  while candidate + offset < needle.len() {
    let current = needle[position + offset];
    let next = needle[candidate + offset];
    let accept = if maximal { next > current } else { next < current };
    let skip = if maximal { next < current } else { next > current };

    if accept {
      position = candidate;
      period = 1;
      candidate += 1;
      offset = 0;
    } else if skip {
      candidate += offset + 1;
      offset = 0;
      period = candidate - position;
    } else if offset + 1 == period {
      candidate += period;
      offset = 0;
    } else {
      offset += 1;
    }
  }

  (position, period)
}

// Chooses the short-period or long-period search and its shift amount.
//
// The first element is true for the short-period case and false for the
// long-period case. The second element is the period or the long-period
// shift, respectively.
func two_way_shift(needle: *[u8], suffix_period: usize, cut: usize) -> (bool, usize) {
  let length = needle.len();
  let right_length = length - cut;
  let large = if cut > right_length { cut } else { right_length };

  if cut * 2 >= length || cut > suffix_period || suffix_period > right_length {
    return (false, large);
  }

  // The short-period case requires `u` to be a suffix of `v[..period]`.
  if !bytes_equal_range(needle, suffix_period, needle, 0, cut) {
    return (false, large);
  }
  (true, suffix_period)
}

func find_two_way(haystack: *[u8], needle: *[u8]) -> Option<usize> {
  let minimal = suffix(needle, false);
  let maximal = suffix(needle, true);
  let minimal_position = minimal.0;
  let maximal_position = maximal.0;
  let suffix_period: usize;
  let cut: usize;

  if minimal_position > maximal_position {
    suffix_period = minimal.1;
    cut = minimal_position;
  } else {
    suffix_period = maximal.1;
    cut = maximal_position;
  }

  let shift = two_way_shift(needle, suffix_period, cut);
  if shift.0 {
    find_short_period(haystack, needle, cut, shift.1)
  } else {
    find_long_period(haystack, needle, cut, shift.1)
  }
}

// Searches with the exact period of the needle.
func find_short_period(
  haystack: *[u8],
  needle: *[u8],
  cut: usize,
  period: usize,
) -> Option<usize> {
  let length = needle.len();
  let mut start: usize = 0;
  let mut shift: usize = 0;

  while start + length <= haystack.len() {
    let mut index = if cut > shift { cut } else { shift };
    while index < length && needle[index] == haystack[start + index] {
      index += 1;
    }

    if index < length {
      start += index - cut + 1;
      shift = 0;
    } else {
      index = cut;
      while index > shift && needle[index] == haystack[start + index] {
        index -= 1;
      }
      if index <= shift && needle[shift] == haystack[start + shift] {
        return Option::Some(start);
      }
      start += period;
      shift = length - period;
    }
  }

  Option::None
}

// Searches with the long-period lower bound.
func find_long_period(
  haystack: *[u8],
  needle: *[u8],
  cut: usize,
  shift: usize,
) -> Option<usize> {
  let length = needle.len();
  let mut start: usize = 0;

  while start + length <= haystack.len() {
    let mut index = cut;
    while index < length && needle[index] == haystack[start + index] {
      index += 1;
    }

    if index < length {
      start += index - cut + 1;
    } else {
      let mut left = cut;
      let mut matched = true;
      while left > 0 {
        left -= 1;
        if needle[left] != haystack[start + left] {
          start += shift;
          matched = false;
          break;
        }
      }
      if matched {
        return Option::Some(start);
      }
    }
  }

  Option::None
}
