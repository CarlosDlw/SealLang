// A trait method may declare a default body. An impl may omit it, inherit it,
// or override it; the default body dispatches symbolically to the concrete
// implementation chosen for `Self`. The same rules apply to associated
// functions without a receiver, including `Self::name()` inside a default body.
// The documented result is 42.

trait Shape {
  // Required: every impl must supply an area.
  func area(self: *Self) -> i32;

  // Default with an override below.
  func name(self: *Self) -> i32 {
    1
  }

  // Default that calls a required method. On a concrete type this becomes a
  // direct call to the override; behind `*dyn Shape` it uses the vtable slot.
  func twice(self: *Self) -> i32 {
    self.area() * 2
  }
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

  func name(self: *Rectangle) -> i32 {
    9
  }
}

impl Shape for Square {
  func area(self: *Square) -> i32 {
    (*self).side * (*self).side
  }
}

// A supertrait default is inherited by the subtrait's impl.
trait Named {
  func base(self: *Self) -> i32 {
    3
  }
}

trait Labeled: Named { }

impl Named for Rectangle { }
impl Labeled for Rectangle { }

// A generic trait default substitutes the trait's own type arguments.
trait Convert<T> {
  func convert(self: *Self) -> T;

  func doubled(self: *Self) -> T {
    self.convert()
  }
}

struct LabeledValue<T> {
  value: T,
}

impl Convert<i32> for LabeledValue<i32> {
  func convert(self: *LabeledValue<i32>) -> i32 {
    (*self).value
  }
}

// Associated functions may have default bodies too. `Self::unit()` inside
// `twice` selects the override below.
trait Factory {
  func unit() -> i32 {
    7
  }

  func twice() -> i32 {
    Self::unit() * 2
  }
}

struct Marker { }

impl Factory for Marker {
  func unit() -> i32 {
    11
  }
}

func dynamic_shape(shape: *dyn Shape) -> i32 {
  shape.twice()
}

func main() -> i32 {
  let rectangle = Rectangle { width: 2, height: 3 };
  let square = Square { side: 1 };
  let label = LabeledValue { value: 5 };

  let from_default = rectangle.twice();
  let from_override = rectangle.name();
  let from_dyn = dynamic_shape(&square);
  let from_supertrait = rectangle.base();
  let from_generic = label.doubled();
  let from_associated = Marker::twice() - Marker::unit();

  from_default + from_override + from_dyn + from_supertrait + from_generic + from_associated
}
