// An array pattern also matches a slice reference (`*[T]` or `*mut [T]`).
// Without `..` the pattern tests the exact runtime length; with `..` it tests
// that the length is at least the number of fixed patterns, then reads the
// prefix from the start and the suffix from the end.
//
// Lengths are runtime values, so coverage reasons about lengths: `[]`, `[only]`,
// and `[first, .., last]` together cover every slice. Binding an element moves
// it, so a slice whose element type is not `Copy` can only be matched with
// literals and `_`.

func head(values: *[i32]) -> i32 {
  match values {
    [] => -1,
    [first, ..] => first,
  }
}

func last(values: *[i32]) -> i32 {
  match values {
    [] => -1,
    [.., last] => last,
  }
}

func pair(values: *[i32]) -> i32 {
  match values {
    [left, right] => left + right,
    _ => 0,
  }
}

func describe(values: *[i32]) -> i32 {
  match values {
    [] => 0,
    [only] => only,
    [first, .., last] if first == last => first,
    [first, .., last] => first + last,
  }
}

func main() -> i32 {
  let empty_data = [0];
  let empty = &empty_data[0..0];

  let one_data = [1];
  let one = &one_data[0..];

  let two_data = [6, 4];
  let two = &two_data[0..];

  let three_data = [5, 9, 6];
  let three = &three_data[0..];

  let total = head(empty)
    + head(one)
    + head(two)
    + last(empty)
    + last(one)
    + last(two)
    + pair(empty)
    + pair(one)
    + pair(two)
    + describe(empty)
    + describe(one)
    + describe(two)
    + describe(three);
  if total == 42 { 42 } else { 1 }
}
