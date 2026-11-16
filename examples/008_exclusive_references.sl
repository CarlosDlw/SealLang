// Exclusive references: `*mut T` and `&mut place`.
//
// A `*mut T` reference permits reading and writing the pointee, and the borrow
// checker treats it as an exclusive loan: no other loan of the same place may
// overlap while the reference is live. `*T` stays read-only, so the two kinds
// never unify.

func bump(count: *mut i32) {
  *count = *count + 1;
}

func main() -> i32 {
  // `&mut value` requires a binding declared with `let mut`.
  let mut value = 41;
  let reference = &mut value;

  // Passing the reference copies the pointer, but the loan stays live with
  // every copy, so `value` remains unavailable until the last use.
  bump(reference);
  *reference = *reference + 1;

  *reference
}
