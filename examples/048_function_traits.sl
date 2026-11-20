// `Fn` reads, `FnMut` writes, and `FnOnce` consumes. The hierarchy is
// `Fn: FnMut: FnOnce`, so one generic framework accepts all three capture
// kinds without boxing. The documented result is 42.

func twice<F: Fn() -> i32>(f: F) -> i32 {
  f() + f()
}

func each<F: FnMut(i32)>(f: F, count: i32) {
  let mut f = f;
  let mut index = 0;
  while index < count {
    f(index);
    index += 1;
  }
}

func consume<F: FnOnce() -> i32>(f: F) -> i32 {
  f()
}

func main() -> i32 {
  let base = 10;
  let doubled = twice(move || base + 1);

  let mut total = 0;
  each(|value: i32| { total += value; }, 4);

  let once = consume(move || 14);
  doubled + total + once
}
