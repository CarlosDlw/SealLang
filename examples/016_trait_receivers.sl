// A method that declares `self: *T` can be called through a `*mut T` receiver:
// the compiler reborrows it shared, and the borrow checker sees that loan.
// A method named `main` is an ordinary method; only a free `main` is the entry
// point.

struct Counter {
  value: i32,
}

trait Peak {
  func peak(self: *Self) -> i32;
}

impl Peak for Counter {
  func peak(self: *Counter) -> i32 {
    (*self).value
  }
}

impl Counter {
  func bump(self: *mut Counter) {
    (*self).value += 1;
  }

  func main(self: *Counter) -> i32 {
    (*self).value * 100
  }
}

func main() -> i32 {
  let mut counter = Counter { value: 40 };
  let pointer = &mut counter;
  pointer.bump();
  // `pointer` is `*mut Counter`; both calls below reborrow or match it.
  let scaled = pointer.main();
  let peak = pointer.peak();
  if scaled == 4100 && peak == 41 { 42 } else { 1 }
}
