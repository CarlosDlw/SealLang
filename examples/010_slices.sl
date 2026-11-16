// A slice is an unsized view over contiguous elements. It is created by
// borrowing a range, and an array reference coerces to a slice reference when
// the context requires one. `*[T]` is a fat pointer holding both the element
// address and the length; `len` reads that length in O(1).

func sum(values: *[i32]) -> i32 {
  let mut index: usize = 0;
  let mut total = 0;
  while index < len(values) {
    total = total + values[index];
    index = index + 1;
  }
  total
}

func first_half(values: *[i32]) -> *[i32] {
  &values[..2]
}

func main() -> i32 {
  let values = [1, 2, 3, 4];

  // `&values` keeps the array type unless the context asks for a slice.
  let all: *[i32] = &values;

  // A range borrows a sub-slice; both bounds are bounds-checked once.
  let middle = &values[1..3];
  let part = first_half(middle);

  // A mutable slice writes through the original storage.
  let mut copy = [10, 20, 30];
  let mut tail: *mut [i32] = &mut copy[1..];
  tail[0] = 99;

  let mut total = sum(all) + sum(middle) + sum(part);
  if len(tail) == 2 {
    total = total + 1;
  }
  total = total + copy[1];
  total
}
