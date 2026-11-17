// A recursive generic type reaches itself through a pointer, so its layout is
// finite. A generic function may call itself with the same type arguments;
// monomorphization reuses the one instance instead of expanding forever.
// The documented result is 42.

enum Option<T> {
  Some(T),
  None,
}

struct Node<T> {
  value: T,
  next: Option<*mut Node<T>>,
}

func sum(node: *mut Node<i32>) -> i32 {
  let mut total = (*node).value;
  match (*node).next {
    Option::Some(next) => {
      total = total + sum(next);
    }
    Option::None => {}
  }
  total
}

func countdown<T>(value: T, steps: i32) -> i32 {
  if steps == 0 {
    0
  } else {
    1 + countdown(value, steps - 1)
  }
}

func main() -> i32 {
  let mut second = Node { value: 20, next: Option::None };
  let mut first = Node { value: 20, next: Option::Some(&mut second) };
  sum(&mut first) + countdown(true, 2)
}
