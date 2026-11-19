// Raw pointers are nullable, copyable values with an explicit unsafe contract.
// This program takes an exclusive raw pointer into an array, advances it,
// writes through it, copies overlapping elements with `copy`, and checks a
// null pointer. Every unsafe operation is inside an `unsafe` block, and the
// documented result is 0; a wrong address or a wrong copy returns a non-zero
// value.

func main() -> i32 {
  let mut values = [10, 20, 30, 40];

  // The cast keeps the provenance of the local array; the pointer is the only
  // exclusive handle used by the unsafe block.
  let base = &mut values[0] as *raw mut i32;

  unsafe {
    let second = base.add(1);
    *second = *second + 2;

    // `copy` allows overlap and lowers to memmove semantics. Copying the first
    // two elements into the last two leaves [10, 22, 10, 22].
    let last = &mut values[2] as *raw mut i32;
    last.copy(base as *raw i32, 2);

    let distance = second.offset_from(base);
    let null: *raw i32 = 0usize as *raw i32;
    let total = *base + *second + *last;

    if null.is_null() && distance == 1 && total == 42 {
      0
    } else {
      1
    }
  }
}
