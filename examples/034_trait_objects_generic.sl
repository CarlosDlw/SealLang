// A generic function with a trait bound can coerce its `*T` parameter to
// `*dyn Shape`. Monomorphization selects the concrete implementation and the
// vtable for each instantiation, so dispatch remains type-checked.
// The documented result is 42.

trait Shape {
  func area(self: *Self) -> i32;
}

struct Rectangle {
  width: i32,
  height: i32,
}

impl Shape for Rectangle {
  func area(self: *Rectangle) -> i32 {
    (*self).width * (*self).height
  }
}

func area_of<T: Shape>(shape: *T) -> i32 {
  let object: *dyn Shape = shape;
  object.area()
}

func main() -> i32 {
  let rectangle = Rectangle { width: 6, height: 7 };
  area_of(&rectangle)
}
