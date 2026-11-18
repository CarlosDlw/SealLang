// A global may hold an aggregate, a reference, a slice, a string, or a trait
// object. Every address inside it is a relocation resolved before the program
// starts, so the image in the object file is complete and no code runs at load
// time. The documented result is 42.

struct Point {
  x: i32,
  y: i32,
}

struct Label {
  tag: i32,
  text: *str,
}

trait Area {
  func area(self: *Self) -> i32;
}

struct Square {
  side: i32,
}

impl Area for Square {
  func area(self: *Square) -> i32 {
    (*self).side * (*self).side
  }
}

const SEED: i32 = 8;

// An aggregate is stored with its language layout.
static POINT: Point = Point { x: 5, y: 24 };
static TABLE: [i32; 4] = [1, 2, 3, 4];

// A field may point at a string literal, which is itself a global image.
static LABEL: Label = Label { tag: 5, text: "seal" };

// A reference to a static is an address, never a copy of the pointee.
static POINT_REF: *Point = &POINT;

// A slice static is a fat pointer: element address plus length.
static VIEW: *[i32] = &TABLE[..];

// A trait object static holds a data pointer and the compiler-built vtable.
static UNIT: Square = Square { side: 1 };
static SHAPE: *dyn Area = &UNIT;

func main() -> i32 {
  // A const has no storage, so `&SEED` materializes one compiler-owned
  // read-only allocation for the program.
  let promoted: *i32 = &SEED;

  let from_slice = VIEW[3];
  let from_pointer = (*POINT_REF).y;
  let from_label = LABEL.tag;
  let from_const = *promoted;
  let from_object = SHAPE.area();

  from_slice + from_pointer + from_label + from_const + from_object
}
