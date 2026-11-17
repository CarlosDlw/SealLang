// A generic struct gets one concrete layout per type argument, so
// `Pair<i32, bool>` and `Pair<Pair<i32, bool>, bool>` are distinct types with
// their own field offsets. Generic impls share their parameters with their
// methods, and an associated function may infer them from its arguments.
// The documented result is 45.

struct Pair<T, U> {
  first: T,
  second: U,
}

struct Grid<T> {
  cells: [T; 3],
  corner: (T, T),
}

impl<T> Pair<T, T> {
  func same(first: T, second: T) -> Pair<T, T> {
    Pair { first: first, second: second }
  }

  func first_value(self: *Pair<T, T>) -> T {
    (*self).first
  }

  // A method may declare its own type parameter after the impl's.
  func mixed<U>(self: *Pair<T, T>, second: U) -> Pair<T, U> {
    Pair { first: (*self).first, second: second }
  }
}

func main() -> i32 {
  let nested = Pair { first: Pair { first: 40, second: false }, second: true };
  let grid = Grid { cells: [1, 2, 1], corner: (1, 0) };
  let doubled = Pair::same(1, 1);
  let combined = doubled.mixed(1);
  nested.first.first + grid.cells[1] + grid.corner.0 + doubled.first_value()
    + combined.second
}
