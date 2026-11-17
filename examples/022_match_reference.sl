// Struct and variant patterns match through references without an explicit
// dereference. The pattern reads the tag and the fields through the reference,
// so every field bound by name must be `Copy`: a non-`Copy` field cannot be
// moved out of storage the reference only borrows.

struct Point {
  x: i32,
  y: i32,
}

enum Shape {
  Point,
  Pair(i32, i32),
  Label { value: i32 },
}

func manhattan(point: *Point) -> i32 {
  match point {
    Point { x, y } => x + y,
  }
}

// A `..` rest ignores every field the arm does not use, also through a
// reference.
func abscissa(point: *Point) -> i32 {
  match point {
    Point { x, .. } => x,
  }
}

func shape_value(shape: *Shape) -> i32 {
  match shape {
    Shape::Point => 0,
    Shape::Pair(left, right) => left + right,
    Shape::Label { value } => value,
  }
}

func main() -> i32 {
  let point = Point { x: 3, y: 4 };
  let pair = Shape::Pair(17, 10);
  let label = Shape::Label { value: 5 };
  manhattan(&point) + shape_value(&pair) + shape_value(&label) + abscissa(&point)
}
