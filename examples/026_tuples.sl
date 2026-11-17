// Tuples are structural values: their type is the ordered list of element
// types, and elements are accessed by position with `.0`, `.1`, and so on.
// A tuple is `Copy` when every element is `Copy`; otherwise assignment moves
// it, and a single element can be moved or borrowed on its own.

struct Point {
  x: i32,
  y: i32,
}

struct Segment {
  ends: ((i32, i32), (i32, i32)),
}

enum Packed {
  Empty,
  Both((i32, i32), i32),
}

func swap(pair: (i32, i32)) -> (i32, i32) {
  (pair.1, pair.0)
}

func make_point() -> (Point, i32) {
  (Point { x: 10, y: 20 }, 4)
}

func main() -> i32 {
  // A literal preserves source order, and `(value)` only groups.
  let mut pair = (1, 2);
  let grouped = (pair.0);
  let single = (7,);

  // Reading and writing fields keeps the place granularity.
  pair.1 = pair.0 + 2;
  let left = pair.0;
  let right = pair.1;

  // Tuple fields can be borrowed independently of their siblings.
  let borrowed = &pair.0;
  pair.1 = 8;
  let borrowed_value = *borrowed;

  // Nested tuples need one access per level.
  let nested = ((1, 2), (3, 4));
  let diagonal = nested.0.1 + nested.1.0;

  // The unit value is the empty tuple, and `()` is also its type.
  let unit: () = ();
  let nothing = match unit {
    () => 0,
  };

  // Tuples returned from functions keep their component types.
  let swapped = swap((5, 6));
  let a = swapped.0;
  let b = swapped.1;
  let made = make_point();
  let first = made.0;
  let second = made.1;

  // A non-`Copy` tuple can be taken apart element by element.
  let points = (Point { x: 1, y: 2 }, Point { x: 3, y: 4 });
  let moved = points.0;
  let kept = points.1;
  let total = moved.x + moved.y + kept.x + kept.y;

  // Tuples are ordinary types inside struct fields, arrays, and enum payloads.
  let segment = Segment { ends: ((1, 2), (3, 4)) };
  let segment_sum = segment.ends.0.1 + segment.ends.1.0;
  let grid = [(1, 2), (3, 4)];
  let grid_sum = grid[0].0 + grid[1].1;
  let mut loop_sum = 0;
  for (a, b) in grid {
    loop_sum += a + b;
  }
  let packed = Packed::Both((6, 7), 8);
  let packed_sum = match packed {
    Packed::Both((x, y), z) => x + y + z,
    Packed::Empty => 0,
  };

  // Tuple patterns dereference a reference and bind its `Copy` elements.
  let coordinate_value = (2, 3);
  let coordinate = &coordinate_value;
  let diagonal2 = match coordinate {
    (cx, cy) => cx + cy,
  };

  // Tuple patterns also work in `if let`.
  let mutable_pair = (Point { x: 5, y: 6 }, 9);
  let extra = if let (p, 9) = mutable_pair {
    p.x + p.y
  } else {
    0
  };

  left
    + right
    + borrowed_value
    + diagonal
    + nothing
    + a
    + b
    + first.x
    + first.y
    + second
    + total
    + single.0
    + grouped
    + diagonal2
    + extra
    + segment_sum
    + grid_sum
    + packed_sum
    + loop_sum
}
