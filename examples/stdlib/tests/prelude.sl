// The test target receives the same std prelude as the package binary.

test func option_variants() {
  let some = Option::Some(41);
  let none = Option::<i32>::None;
  assert(match some { Option::Some(value) => value, Option::None => 0 } == 41);
  assert(match none { Option::Some(_) => 1, Option::None => 0 } == 0);
}

test func result_variants() {
  let ok = Result::<i32, i32>::Ok(1);
  let err = Result::<i32, i32>::Err(2);
  assert(match ok { Result::Ok(value) => value, Result::Err(_) => 0 } == 1);
  assert(match err { Result::Ok(_) => 0, Result::Err(code) => code } == 2);
}
