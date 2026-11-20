// A closure borrows its captures by default; `move` stores them in the
// environment by value. A nested closure reads through the outer environment,
// and the documented result is 42.

func main() -> i32 {
  let base = 10;
  let by_ref = |value: i32| value + base;
  let by_value = move |value: i32| value + base;
  let nested = || {
    let inner = || base + 2;
    inner()
  };

  by_ref(5) + by_value(5) + nested()
}
