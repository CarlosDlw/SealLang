// `Copy` is opt-in for nominal aggregates. This program declares a struct, an
// enum, a conditional generic wrapper, a struct holding a reference, and a
// struct holding a raw pointer, then copies each of them through the language
// operations that consume values by default.
//
// The documented result is 42. The calculation adds one term from each use:
//
//   match point (12) + passed field (2) + indexed element (1) + loop body (2)
//   + wrapper (3) + enum (4) + static (5) + reference holder (6)
//   + raw holder (7) + no-op after drop (0) + source after loop (0)
//
// A source that is still usable after a copy contributes a zero term, so the
// final value also proves that the copy did not move it.

struct Point {
  x: i32,
  y: i32,
}

impl Copy for Point {}

enum Direction {
  North,
  East,
}

impl Copy for Direction {}

struct Wrapper<T> {
  value: T,
}

impl<T: Copy> Copy for Wrapper<T> {}

struct Holder {
  value: *i32,
}

impl Copy for Holder {}

struct RawHolder {
  value: *raw i32,
}

impl Copy for RawHolder {}

static ORIGIN: Point = Point { x: 5, y: 0 };

func copy_value(point: Point) -> Point {
  point
}

func read_holder() -> i32 {
  let number = 6;
  let holder = Holder { value: &number };
  let copy = holder;
  *copy.value
}

func read_raw_holder() -> i32 {
  let number = 7;
  let pointer: *raw i32 = &number as *raw i32;
  let holder = RawHolder { value: pointer };
  let copy = holder;
  unsafe { *copy.value }
}

func main() -> i32 {
  let point = Point { x: 10, y: 2 };
  let assigned = point;
  let passed = copy_value(point);
  let matched = match assigned {
    Point { x, y } => x + y,
  };
  let field = passed.y;

  let points = [Point { x: 1, y: 1 }; 2];
  let indexed = points[0];
  let mut loop_total = 0;
  for item in points {
    loop_total += item.y;
  }
  // The source is still usable because each iterated item was copied.
  let after_loop = points[1].x - 1;

  let wrapped = Wrapper { value: Point { x: 3, y: 0 } };
  let copied_wrapper = wrapped;
  let wrapper_value = copied_wrapper.value.x;

  let direction = Direction::East;
  let copied_direction = direction;
  let direction_value = match copied_direction {
    Direction::North => 0,
    Direction::East => 4,
  };

  let from_static = ORIGIN;
  let static_value = from_static.x;

  let holder_value = read_holder();
  let raw_value = read_raw_holder();

  // Dropping a `Copy` value consumes nothing and runs no destructor.
  drop(point);
  let after_drop = point.x - 10;

  matched
    + field
    + indexed.x
    + after_loop
    + loop_total
    + wrapper_value
    + direction_value
    + static_value
    + holder_value
    + raw_value
    + after_drop
}
