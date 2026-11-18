// A custom source implements `IntoIterator<Iter, Item>` and returns a cursor
// that implements `Iterator<Item>`. A type that already implements `Iterator`
// can also be used directly; `IntoIterator` always takes precedence when both
// are available. The documented result is 42.

struct Series {
  start: i32,
  end: i32,
}

struct Cursor {
  next: i32,
  end: i32,
}

impl Iterator<i32> for Cursor {
  func has_next(self: *Cursor) -> bool {
    (*self).next < (*self).end
  }

  func next(self: *mut Cursor) -> i32 {
    let value = (*self).next;
    (*self).next += 1;
    value
  }
}

impl IntoIterator<Cursor, i32> for Series {
  func iter(self: Series) -> Cursor {
    Cursor { next: self.start, end: self.end }
  }
}

// This source is its own iterator, so the direct fallback drives the loop.
struct Single {
  value: i32,
  remaining: i32,
}

impl Iterator<i32> for Single {
  func has_next(self: *Single) -> bool {
    (*self).remaining > 0
  }

  func next(self: *mut Single) -> i32 {
    (*self).remaining = 0;
    (*self).value
  }
}

func main() -> i32 {
  let series = Series { start: 10, end: 13 };
  let mut total = 0;

  // One `iter` call creates the cursor; `has_next` and `next` drive the loop.
  for value in series {
    total += value;
  }

  // The direct fallback uses the source value itself as the iterator.
  let single = Single { value: 9, remaining: 1 };
  for value in single {
    total += value;
  }

  total
}
