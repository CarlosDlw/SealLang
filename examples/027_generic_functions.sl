// A generic function is checked once, with a rigid type parameter, and one
// concrete body is generated per type argument used at a call site.
// Type arguments are explicit with `::<...>` or inferred from the arguments.
// The documented result is 42.

func identity<T>(value: T) -> T {
  value
}

// `T: Copy` is a structural bound: the instantiated type must be `Copy`, so
// passing `value` twice is allowed and each use copies.
func duplicate<T: Copy>(value: T) -> (T, T) {
  (value, value)
}

func main() -> i32 {
  let explicit = identity::<i32>(20);
  let inferred = identity(21);
  let pair = duplicate(inferred);
  let flag = identity(true);
  let bonus = if flag { 1 } else { 0 };
  explicit + pair.0 + bonus
}
