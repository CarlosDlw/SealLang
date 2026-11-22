// `mem::forget` consumes a value without running its destructor. The first
// token is forgotten, so the destructor counter stays zero; the second token is
// dropped normally and increments it. A `Copy` value remains usable after being
// forgotten because forgetting a copy has nothing to release.

struct Token {
  value: i32,
}

static mut DROPS: i32 = 0;

impl Drop for Token {
  func drop(self: *mut Token) {
    unsafe {
      DROPS += 1;
    }
  }
}

func make(value: i32) -> Token {
  Token { value: value }
}

func main() -> i32 {
  let forgotten = make(1);
  std::mem::forget(forgotten);
  if unsafe { DROPS } != 0 {
    return 1;
  }

  let dropped = make(2);
  drop(dropped);
  if unsafe { DROPS } != 1 {
    return 2;
  }

  let number = 7;
  std::mem::forget(number);
  if number != 7 {
    return 3;
  }

  0
}
