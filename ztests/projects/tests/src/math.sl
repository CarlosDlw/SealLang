module math;

internal func triple(value: i32) -> i32 {
  value * 3
}

test func module_tests_are_discovered() {
  assert(triple(3) == 9);
}
