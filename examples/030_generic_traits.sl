// A trait may declare its own type parameters, and an impl may be generic
// over them. A bound such as `T: Convert<U>` proves at type-checking time and
// resolves at monomorphization time to one direct call; there is no vtable,
// no dictionary argument, and no runtime type metadata.
// The documented result is 42.

trait Convert<T> {
  func convert(self: *Self) -> T;
}

struct Labeled<T> {
  label: T,
}

impl<T: Copy> Convert<T> for Labeled<T> {
  func convert(self: *Labeled<T>) -> T {
    (*self).label
  }
}

struct Counter {
  value: i32,
}

impl Convert<i32> for Counter {
  func convert(self: *Counter) -> i32 {
    (*self).value
  }
}

// A `where` clause is equivalent to an inline bound. `U` is inferred from the
// expected return type and the bound's trait argument.
func read<T, U>(value: *T) -> U where T: Convert<U> {
  value.convert()
}

func main() -> i32 {
  let labeled = Labeled { label: 40 };
  let counter = Counter { value: 2 };
  let first: i32 = read(&labeled);
  let second: i32 = read(&counter);
  first + second
}
