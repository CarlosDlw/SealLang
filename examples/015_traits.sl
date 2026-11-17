// Traits declare method signatures and each impl supplies a concrete function.
// Every call resolves statically: there is no vtable and no trait object.
// A signature writes `Self` for the implementing type.

struct Rectangle {
  width: i32,
  height: i32,
}

struct Square {
  side: i32,
}

trait Area {
  func area(self: *Self) -> i32;
  func doubled(self: *Self) -> i32;
}

trait Named {
  // An associated function has no receiver and is called as `Type::name()`.
  func weight() -> i32;
}

impl Area for Rectangle {
  func area(self: *Rectangle) -> i32 {
    (*self).width * (*self).height
  }

  func doubled(self: *Rectangle) -> i32 {
    self.area() * 2
  }
}

impl Area for Square {
  func area(self: *Square) -> i32 {
    (*self).side * (*self).side
  }

  func doubled(self: *Square) -> i32 {
    self.area() * 2
  }
}

impl Named for Square {
  func weight() -> i32 {
    3
  }
}

func main() -> i32 {
  let rectangle = Rectangle { width: 4, height: 5 };
  let square = Square { side: 3 };
  rectangle.doubled() + square.area() + Square::weight()
}
