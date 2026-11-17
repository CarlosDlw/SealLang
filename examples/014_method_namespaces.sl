// Methods and associated functions live in a per-type namespace. Two structs
// may both define `new` and `area`; calls select the impl by receiver type.

struct Rect {
  width: i32,
  height: i32,
}

struct Circle {
  radius: i32,
}

impl Rect {
  func new(width: i32, height: i32) -> Rect {
    Rect { width: width, height: height }
  }

  func area(self: *Rect) -> i32 {
    (*self).width * (*self).height
  }
}

impl Circle {
  func new(radius: i32) -> Circle {
    Circle { radius: radius }
  }

  func area(self: *Circle) -> i32 {
    (*self).radius * (*self).radius
  }
}

func main() -> i32 {
  let rect = Rect::new(3, 4);
  let circle = Circle::new(2);
  rect.area() + circle.area()
}
