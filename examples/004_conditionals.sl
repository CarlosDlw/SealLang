// Conditionals are expressions: a block is the value of the branch that runs,
// so `if` can appear wherever a value is expected.
func classify(value: i32) -> i32 {
  if value < 0 {
    -1
  } else if value == 0 {
    0
  } else {
    1
  }
}

// An `if` without `else` has the unit type and is used as a statement.
func guard(value: i32) -> i32 {
  if value < 0 {
    return 0;
  }

  value
}

func main() -> i32 {
  let a = classify(-5);
  let b = classify(0);
  let c = classify(7);

  // `if` in expression position, with nested branches.
  let biggest = if a > b {
    if a > c {
      a
    } else {
      c
    }
  } else if b > c {
    b
  } else {
    c
  };

  // `if` as a statement; without `else`, both branches are unit.
  if biggest == 0 {
    return 1;
  }

  guard(biggest) + b
}
