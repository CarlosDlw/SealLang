// `match` treats a braced block as an ordinary expression, so arms, `let`
// initializers, and nested positions can compute a value with statements.
//
// Patterns go beyond enum variants: a struct pattern destructures named fields,
// a range pattern compares against an interval, `name @ pattern` binds the whole
// value while the subpattern narrows it, and an array pattern matches every
// element of a fixed-size array by position; `..` skips the elements in
// between the fixed patterns.

enum Shape {
  Point,
  Circle(i32),
  Label { text: char, size: i32 },
}

struct Point {
  x: i32,
  y: i32,
}

func bucket(value: i32) -> i32 {
  match value {
    -10..=-1 => -1,
    0..=9 => {
      let tens = 1;
      tens
    }
    10..=19 => 2,
    n @ 20..=29 => n / 10,
    _ => 0,
  }
}

func classify_char(letter: char) -> i32 {
  match letter {
    'a'..='z' => 1,
    'A'..='Z' => 2,
    _ => 0,
  }
}

func manhattan(point: Point) -> i32 {
  match point {
    Point { x: 0, y } => y,
    Point { x, y: 0 } => x,
    Point { x, y } if x == y => x * 10,
    Point { x, y } => x + y,
  }
}

func first_two(values: [i32; 3]) -> i32 {
  match values {
    [first, 0, 0] => first,
    [0, second, 0] => second,
    [.., third] => third,
  }
}

func describe(shape: Shape) -> i32 {
  match shape {
    Shape::Point => { 0 },
    Shape::Circle(radius) if radius > 10 => {
      let doubled = radius * 2;
      doubled
    }
    Shape::Circle(radius) => radius,
    Shape::Label { text: 'a'..='m', size } => size + 100,
    Shape::Label { text: _, size } => size + 200,
  }
}

func main() -> i32 {
  let base = {
    let one = 1;
    one
  };
  let total = base
    + bucket(-5)
    + bucket(5)
    + bucket(15)
    + bucket(25)
    + bucket(100)
    + classify_char('q')
    + classify_char('Q')
    + classify_char('5')
    + manhattan(Point { x: 0, y: 3 })
    + manhattan(Point { x: 4, y: 0 })
    + manhattan(Point { x: 5, y: 5 })
    + manhattan(Point { x: 2, y: 3 })
    + first_two([7, 0, 0])
    + first_two([0, 8, 0])
    + first_two([1, 2, 9])
    + describe(Shape::Point)
    + describe(Shape::Circle(20))
    + describe(Shape::Circle(4))
    + describe(Shape::Label { text: 'c', size: 5 })
    + describe(Shape::Label { text: 'z', size: 6 });
  if total == 449 { 42 } else { 1 }
}
