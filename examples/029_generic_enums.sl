// A generic enum is one variant set with a payload type per instantiation.
// The tag and payload layout are computed per type argument, and matching
// reads the tag and projects the payload at the instance's offsets.
// The documented result is 42.

enum Option<T> {
  Some(T),
  None,
}

enum Result<T, E> {
  Ok(T),
  Err(E),
}

func unwrap_or<T: Copy>(value: Option<T>, fallback: T) -> T {
  match value {
    Option::Some(inner) => inner,
    Option::None => fallback,
  }
}

func fold(value: Result<i32, i32>, fallback: i32) -> i32 {
  match value {
    Result::Ok(inner) => inner,
    Result::Err(code) => fallback + code,
  }
}

func main() -> i32 {
  let some = Option::Some(40);
  let none = Option::<i32>::None;
  let ok = Result::<i32, i32>::Ok(2);
  let err = Result::<i32, i32>::Err(0);
  unwrap_or(some, 0) + unwrap_or(none, 0) + fold(ok, 0) + fold(err, 0)
}
