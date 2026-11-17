// `if let` evaluates its scrutinee once and runs the body only when the
// pattern matches. Bindings exist only in that body. The pattern may be
// refutable, and `else if let` can be mixed with ordinary `else if`.
//
// Patterns work exactly as in `match`, including struct and slice patterns and
// matching a reference without an explicit dereference.

enum Option {
  None,
  Some(i32),
}

struct Point {
  x: i32,
  y: i32,
}

func unwrap_or(value: Option, fallback: i32) -> i32 {
  if let Option::Some(inner) = value {
    inner
  } else {
    fallback
  }
}

func category(value: i32) -> i32 {
  if let 0 = value {
    10
  } else if let 1..=9 = value {
    20
  } else {
    30
  }
}

func point_sum(point: *Point) -> i32 {
  if let Point { x, y } = point {
    x + y
  } else {
    0
  }
}

func has_value(value: *Option) -> bool {
  if let Option::Some(_) = value { true } else { false }
}

func record(value: Option) -> bool {
  let mut seen = false;
  if let Option::Some(_) = value {
    seen = true;
  }
  seen
}

func either(value: i32) -> i32 {
  if let 0 | 1 = value { 100 } else { 0 }
}

func main() -> i32 {
  let none = Option::None;
  let some = Option::Some(4);
  let point = Point { x: 20, y: 22 };

  let mut total = unwrap_or(none, 1) + unwrap_or(Option::Some(4), 0);
  total += category(0) + category(5) + category(20);
  total += point_sum(&point);
  if has_value(&some) {
    total += 1;
  }
  if record(some) {
    total += 1;
  }
  if either(0) == 100 {
    total -= 100;
  }
  if let 9 = total { 42 } else { total }
}
