// A supertrait method is part of the principal object's vtable. Calling
// `value` through `*dyn Scaled` dispatches the implementation required by
// `Value` without a second pointer or a runtime type search.
// The documented result is 44.

trait Value {
  func value(self: *Self) -> i32;
}

trait Scaled: Value {
  func scaled(self: *Self) -> i32;
}

struct Number {
  value: i32,
}

impl Value for Number {
  func value(self: *Number) -> i32 {
    (*self).value
  }
}

impl Scaled for Number {
  func scaled(self: *Number) -> i32 {
    (*self).value * 10
  }
}

func main() -> i32 {
  let number = Number { value: 4 };
  let object: *dyn Scaled = &number;
  object.scaled() + object.value()
}
