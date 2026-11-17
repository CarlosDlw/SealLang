// `match` is an expression. It evaluates its scrutinee once and selects the
// first arm whose pattern matches and whose guard succeeds.
//
// Patterns cover enum variants, positional and named fields, bindings, scalar
// literals, alternatives with `|`, and the wildcard `_`. Every match must be
// exhaustive, and an arm that can never be selected is an error.
//
// A guard runs after the pattern matches. It may use `Copy` bindings, because a
// failed guard must leave the scrutinee available for the next arm.

enum Command {
  Stop,
  Move(i32),
  Write { code: i32 },
  Pair(i32, i32),
}

func classify(command: Command) -> i32 {
  match command {
    Command::Stop => 0,
    Command::Move(0) | Command::Move(1) => 1,
    Command::Move(step) if step > 10 => 2,
    Command::Move(step) => step,
    Command::Write { code: 7 } => 7,
    Command::Write { code } => code,
    Command::Pair(left, right) => left + right,
  }
}

func main() -> i32 {
  let stop = classify(Command::Stop);
  let one = classify(Command::Move(1));
  let large = classify(Command::Move(20));
  let small = classify(Command::Move(5));
  let exact = classify(Command::Write { code: 7 });
  let other = classify(Command::Write { code: 3 });
  let pair = classify(Command::Pair(22, 2));

  if stop + one + large + small + exact + other + pair == 42 {
    42
  } else {
    1
  }
}
