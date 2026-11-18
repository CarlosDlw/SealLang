// One trait-object pointer can hold either implementation. The data pointer
// addresses the concrete value and the vtable selects `area` for that type.
// The documented result is 42.

trait Shape {
  func area(self: *Self) -> i32;
}

struct Rectangle {
  width: i32,
  height: i32,
}

struct Square {
  side: i32,
}

impl Shape for Rectangle {
  func area(self: *Rectangle) -> i32 {
    (*self).width * (*self).height
  }
}

impl Shape for Square {
  func area(self: *Square) -> i32 {
    (*self).side * (*self).side
  }
}

func area(shape: *dyn Shape) -> i32 {
  shape.area()
}

func main() -> i32 {
  let rectangle = Rectangle { width: 13, height: 2 };
  let square = Square { side: 4 };
  let first: *dyn Shape = &rectangle;
  let second: *dyn Shape = &square;
  area(first) + area(second)
}
