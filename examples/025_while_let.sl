// `while let` reevaluates its scrutinee and retests the pattern before every
// iteration. When the pattern matches, its bindings are available in the body;
// when it fails, the loop exits. `continue` reevaluates the scrutinee, and
// `break` leaves immediately.

enum Option {
  None,
  Some(i32),
}

func next(remaining: i32) -> Option {
  if remaining > 0 { Option::Some(remaining) } else { Option::None }
}

func sum_down(start: i32) -> i32 {
  let mut remaining = start;
  let mut total = 0;
  while let Option::Some(value) = next(remaining) {
    total += value;
    remaining -= 1;
  }
  total
}

func bounded() -> i32 {
  let mut value = 0;
  let mut total = 0;
  while let 0..=5 = value {
    value += 1;
    if value == 3 {
      continue;
    }
    if value > 5 {
      break;
    }
    total += value;
  }
  total
}

func sum_slice(values: *[i32]) -> i32 {
  let mut rest = values;
  let mut total = 0;
  while let [first, ..] = rest {
    total += first;
    rest = &rest[1..];
  }
  total
}

func main() -> i32 {
  let values = [4, 5, 6];
  sum_down(5) + bounded() + sum_slice(&values)
}
