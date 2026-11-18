// A `const` is a value, not storage: every read copies the bytes that constant
// evaluation produced. A `static` is one allocation for the whole program, so it
// can be addressed and read at runtime. Both are initialized before the program
// starts, and a `const func` may be called from either context.
// The documented result is 42.

struct Rect {
  width: i32,
  height: i32,
}

// A constant aggregate: the entire memory image is evaluated once.
const UNIT: Rect = Rect { width: 3, height: 4 };

// Constant arithmetic may read another constant, including its fields.
const AREA: i32 = UNIT.width * UNIT.height;

// A static has storage, an address, and the alignment of its type.
static EDGE: i32 = 18;

// `const func` runs at compile time in a constant context and at runtime here.
const func doubled(value: i32) -> i32 {
  value * 2
}

func main() -> i32 {
  // A local const is evaluated at compile time and is visible to the rest of
  // its block only.
  const TWICE_WIDTH: i32 = UNIT.width * 2;

  let from_const = AREA;
  let from_static = EDGE;
  let from_function = doubled(TWICE_WIDTH);

  from_const + from_static + from_function
}
