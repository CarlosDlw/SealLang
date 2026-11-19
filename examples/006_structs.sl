// Aggregates are nominal and own their fields. This `Point` does not implement
// `Copy`, so assignment and argument passing move it; a `Copy` field can still
// be read by value. Field access works on places, and a borrow can select one
// field.

struct Point {
  x: i32,
  y: i32,
}

struct Segment {
  start: Point,
  end: Point,
}

func width_sum(segment: Segment) -> i32 {
  segment.end.x - segment.start.x + segment.end.y - segment.start.y
}

// Reading through a reference is explicit; `*T` is read-only.
func sum_point(point: *Point) -> i32 {
  (*point).x + (*point).y
}

// Borrowing a field through a parameter's reference is accepted: the loan
// points outside this frame, so it cannot dangle.
func first_x(segment: *Segment) -> *i32 {
  &(*segment).start.x
}

func main() -> i32 {
  let origin = Point { x: 0, y: 0 };
  let corner = Point { y: 8, x: 5 };

  // `Point` owns its fields, so this moves `origin`.
  let mut base = origin;

  // Reading a `Copy` field does not move the struct.
  let ax = base.x;

  // Borrowing one field leaves its siblings writable.
  let bx = &base.x;
  base.y = corner.y;
  let bx_value = *bx;

  // Whole-struct assignment initializes every field at once.
  let mut target = Point { x: 0, y: 0 };
  target = Point { x: bx_value, y: base.y };
  let target_sum = sum_point(&target);

  // Field-by-field initialization needs `mut` and a declared type.
  let mut pending: Point;
  pending.x = target.x + 1;
  pending.y = target.y + 1;

  // `base` and `pending` move into the segment after their last borrow.
  let segment = Segment { start: base, end: pending };
  let width = width_sum(segment);

  // A reference returned through a parameter borrows data that outlives it.
  let other = Segment { start: target, end: Point { x: 1, y: 2 } };
  let left = first_x(&other);

  ax + bx_value + target_sum + width + *left
}
