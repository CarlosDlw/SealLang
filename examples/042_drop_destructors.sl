// A type with `impl Drop` runs its destructor when a value leaves its scope;
// `drop(value)` releases one early. Fields, the active enum variant, and array
// elements drop in the documented order, and a `static` is never released.
//
// Every destructor appends its tag to `ORDER`, so the final value spells the
// order in which they ran:
//
//   scoped(): the pair destructor (9) then its field (2), then the active
//             variant of the slot (1)                 -> 921
//   main():   drop(token) (3)                         -> 9213
//   for:      the item at `break` (4) then the rest (5) -> 921345
//
// `main` returns 0 when the order matches and 1 otherwise, which is the
// documented result.

struct Token {
  tag: i32,
}

impl Drop for Token {
  func drop(self: *mut Token) {
    unsafe {
      ORDER = ORDER * 10 + (*self).tag;
    }
  }
}

enum Slot {
  Value(Token),
  Empty,
}

struct Pair<T> {
  value: T,
}

impl<T> Drop for Pair<T> {
  func drop(self: *mut Pair<T>) {
    unsafe {
      ORDER = ORDER * 10 + 9;
    }
  }
}

static mut ORDER: i32 = 0;

const EXPECTED: i32 = 921345;

func scoped() {
  // `pair` was declared last, so it drops first; its destructor runs before
  // the field it owns.
  let slot = Slot::Value(Token { tag: 1 });
  let pair = Pair { value: Token { tag: 2 } };
}

func main() -> i32 {
  scoped();

  // The destructor runs here, not at the end of `main`.
  let token = Token { tag: 3 };
  drop(token);

  // `break` drops the current item, then the loop drops the elements it never
  // moved into the pattern.
  for item in [Token { tag: 4 }, Token { tag: 5 }] {
    break;
  }

  if unsafe { ORDER } == EXPECTED {
    0
  } else {
    1
  }
}
