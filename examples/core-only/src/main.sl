// The core prelude puts `Option` and `Result` in scope without the hosted
// standard library. Variants stay qualified.

func option_or(value: Option<i32>, fallback: i32) -> i32 {
  match value {
    Option::Some(inner) => inner,
    Option::None => fallback,
  }
}

func result_or(value: Result<i32, i32>, fallback: i32) -> i32 {
  match value {
    Result::Ok(inner) => inner,
    Result::Err(code) => fallback + code,
  }
}

func main() -> i32 {
  let some = Option::Some(41);
  let none = Option::<i32>::None;
  let ok = Result::<i32, i32>::Ok(1);
  let err = Result::<i32, i32>::Err(0);

  if option_or(some, 0) + option_or(none, 0) + result_or(ok, 0) + result_or(err, 0) == 42 {
    0
  } else {
    1
  }
}
