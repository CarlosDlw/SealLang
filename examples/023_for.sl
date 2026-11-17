// `for` iterates an integer range, an array by value, or a reference to an
// array or slice. The pattern must match every element; `continue` still
// advances the range or the index.

func sum_below(limit: i32) -> i32 {
  let mut total = 0;
  for value in 0..limit {
    total += value;
  }
  total
}

func sum_through(limit: i32) -> i32 {
  let mut total = 0;
  for value in 0..=limit {
    if value == 0 {
      continue;
    }
    total += value;
  }
  total
}

func sum_array(values: [i32; 4]) -> i32 {
  let mut total = 0;
  for value in values {
    total += value;
  }
  total
}

func sum_indexed(values: *[i32]) -> i32 {
  let mut total = 0;
  for index in 0..len(values) {
    total += values[index];
  }
  total
}

func sum_shared(values: *[i32]) -> i32 {
  let mut total = 0;
  for value in values {
    total += *value;
  }
  total
}

func bump(values: *mut [i32; 2]) {
  for value in values {
    *value += 1;
  }
}

func count_until(values: *[i32]) -> i32 {
  let mut count = 0;
  for value in values {
    if *value < 0 {
      continue;
    }
    if *value > 3 {
      break;
    }
    count += 1;
  }
  count
}

func nested() -> i32 {
  let mut total = 0;
  for row in 0..3 {
    for column in 0..3 {
      if column == 1 {
        continue;
      }
      if row == 2 {
        break;
      }
      total += 1;
    }
  }
  total
}

func main() -> i32 {
  let values = [1, 2, 3, 4];
  let mut mutable = [10, 20];

  let mut total = sum_below(5) + sum_through(4) + sum_array(values);
  total += sum_indexed(&values) + sum_shared(&values);

  bump(&mut mutable);
  total + mutable[0] + mutable[1] + count_until(&values) + nested()
}
