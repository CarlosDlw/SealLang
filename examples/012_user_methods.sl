// Inherent impl blocks attach associated constructors and instance methods to
// one nominal struct. The receiver type is explicit in every method signature.

struct Point {
  x: i32,
  y: i32,
}

impl Point {
  func new(x: i32, y: i32) -> Point {
    Point { x: x, y: y }
  }

  func sum(self: *Point) -> i32 {
    (*self).x + (*self).y
  }

  func shift(self: *mut Point, dx: i32, dy: i32) {
    (*self).x += dx;
    (*self).y += dy;
  }

  func into_x(self: Point) -> i32 {
    self.x
  }
}

func main() -> i32 {
  let mut point = Point::new(10, 20);
  point.shift(1, 2);

  let total = point.sum();
  let origin = Point::new(3, 4);
  let copied = origin.into_x();

  total + copied + Point::new(5, 6).sum()
}
