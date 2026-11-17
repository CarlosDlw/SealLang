// Receivers participate in borrow checking. A shared receiver borrows for the
// duration of the call, and an exclusive receiver requires writable storage.

struct Counter {
  value: i32,
}

impl Counter {
  func new(start: i32) -> Counter {
    Counter { value: start }
  }

  func add(self: *mut Counter, amount: i32) {
    (*self).value += amount;
  }

  func read(self: *Counter) -> i32 {
    (*self).value
  }

  func finish(self: Counter) -> i32 {
    self.value * 2
  }
}

func main() -> i32 {
  let mut counter = Counter::new(5);
  counter.add(3);

  let view: *Counter = &counter;
  let observed = view.read();

  counter.add(observed);

  let finished = counter.finish();
  finished + 20
}
