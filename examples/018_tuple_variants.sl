// A variant may carry positional fields instead of named ones: `Some(i32)`
// declares one unnamed field, and `Rect(i32, i32)` declares two. Construction
// uses parentheses, and arguments bind to fields in declaration order.
//
// The three variant forms are exclusive. A unit variant is the bare path
// `Option::None`; a positional variant is `Shape::Circle(7)`; a named variant
// keeps the braced form `Shape::Circle { radius: 7 }`.
//
// Positional fields have no names, so they cannot be read without `match`, just
// like named payload fields. This program checks construction, moves, and the
// tagged layout through method calls and a struct field.

enum Option {
  None,
  Some(i32),
}

enum Shape {
  Empty,
  Circle(i32),
  Rect(i32, i32),
}

struct Canvas {
  shape: Shape,
  count: i32,
}

impl Option {
  // An associated function can build a positional variant.
  func some(value: i32) -> Option {
    Option::Some(value)
  }
}

func area(shape: Shape) -> i32 {
  // The enum value is moved into this function and owns its payload.
  1
}

func discard_option(option: Option) -> i32 {
  // A positional payload is owned by the enum value and moves with it.
  1
}

func store(shape: Shape, count: i32) -> Canvas {
  Canvas { shape: shape, count: count }
}

func main() -> i32 {
  let none = Option::None;
  let some = Option::some(20);
  let circle = Shape::Circle(7);
  let rect = Shape::Rect(3, 4);

  // The struct owns `circle`; constructing it moves the enum into `canvas`.
  let mut canvas = store(circle, 2);
  canvas.count += 1;

  // Unit and positional values move into the calls the same way.
  let first = discard_option(none);
  let second = discard_option(some);
  let third = area(rect);

  if canvas.count == 3 && first + second + third == 3 { 42 } else { 1 }
}
