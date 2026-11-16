// `while` repeats its body while the condition is `true`. The body is a block,
// and `break`/`continue` are statements, so they end with `;`.
func sum_odd_below(limit: i32) -> i32 {
  let mut total = 0;
  let mut current = 0;

  while current < limit {
    current++;

    if current % 2 == 0 {
      continue;
    }

    total += current;
  }

  total
}

func main() -> i32 {
  let mut attempts = 0;

  // `while true` with `break` is the loop-with-exit form.
  while true {
    attempts++;

    if attempts >= 3 {
      break;
    }
  }

  // Nested loops: `break` and `continue` act on the innermost loop.
  let mut row = 0;
  let mut pairs = 0;

  while row < 3 {
    let mut column = 0;

    while column < 3 {
      column++;

      if column == 2 {
        continue;
      }

      if row == 2 {
        break;
      }

      pairs++;
    }

    row++;
  }

  sum_odd_below(10) + attempts + pairs
}
