public func double(value: i32) -> i32 {
  value * 2
}

func hidden() -> i32 {
  7
}

test func doubles_a_positive_number() {
  assert(double(2) == 4);
}

test func doubles_zero() {
  assert(double(0) == 0, "zero must stay zero");
}

test func unit_sees_private_items() {
  assert(hidden() == 7);
}

test func reports_a_failure() {
  assert(double(2) == 5, "the fixture fails on purpose");
}
