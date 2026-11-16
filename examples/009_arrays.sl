// Fixed-size arrays own their elements. An array is `Copy` exactly when its
// element type is `Copy`, so copying `values` below leaves the original usable.
// Indices are `usize`; a dynamic index is checked at runtime before it is used.

struct Point {
  x: i32,
  y: i32,
}

struct Row {
  values: [i32; 2],
}

func sum(values: [i32; 4]) -> i32 {
  values[0] + values[1] + values[2] + values[3]
}

func main() -> i32 {
  // A literal keeps source order, and constant indices need no runtime check.
  let mut values = [1, 2, 3, 4];
  values[1] = 20;

  // Dynamic indices are evaluated once and bounds-checked before the access.
  let index: usize = 2;
  values[index] = values[index] + 1;

  // `[value; N]` evaluates the value once and copies it into every element.
  let repeated = [5; 3];

  // Empty arrays need a known element type; they occupy no storage.
  let empty: [i32; 0] = [];

  // Arrays nest, and a field may itself be an array.
  let row = Row { values: [7, 8] };

  // An array of non-`Copy` structs moves its elements one by one.
  let mut points = [Point { x: 1, y: 2 }, Point { x: 3, y: 4 }];
  points[0].x = 10;
  let point_total = points[0].x + points[0].y + points[1].x + points[1].y;

  // Arrays of `Copy` elements can be copied as a whole.
  let copy = values;

  sum(copy) + repeated[0] + repeated[1] + repeated[2] + row.values[1] + point_total
}
