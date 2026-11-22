// `mem::forget` is safe, consumes non-`Copy` values, and leaves `Copy` values
// usable. The destructor counter proves that forgetting suppresses drop glue.

use core::mem::forget as core_forget;
use std::mem::forget;

struct Token {
  value: i32,
}

struct ReleaseToken {
  value: i32,
}

struct ExplicitToken {
  value: i32,
}

struct GenericToken {
  value: i32,
}

struct Inner {
  value: i32,
}

struct Outer {
  inner: Inner,
}

static mut TOKEN_DROPS: i32 = 0;
static mut RELEASE_DROPS: i32 = 0;
static mut EXPLICIT_DROPS: i32 = 0;
static mut GENERIC_DROPS: i32 = 0;
static mut FIELD_DROPS: i32 = 0;

impl Drop for Token {
  func drop(self: *mut Token) {
    unsafe {
      TOKEN_DROPS += 1;
    }
  }
}

impl Drop for ReleaseToken {
  func drop(self: *mut ReleaseToken) {
    unsafe {
      RELEASE_DROPS += 1;
    }
  }
}

impl Drop for ExplicitToken {
  func drop(self: *mut ExplicitToken) {
    unsafe {
      EXPLICIT_DROPS += 1;
    }
  }
}

impl Drop for GenericToken {
  func drop(self: *mut GenericToken) {
    unsafe {
      GENERIC_DROPS += 1;
    }
  }
}

impl Drop for Inner {
  func drop(self: *mut Inner) {
    unsafe {
      FIELD_DROPS += 1;
    }
  }
}

func consume<T>(value: T) {
  forget(value);
}

test func forget_suppresses_drop_and_leaves_copy_usable() {
  let token = Token { value: 1 };
  forget(token);
  assert(unsafe { TOKEN_DROPS } == 0);

  let dropped = Token { value: 2 };
  drop(dropped);
  assert(unsafe { TOKEN_DROPS } == 1);

  let number = 9;
  forget(number);
  assert(number == 9);

  core_forget(Token { value: 3 });
  assert(unsafe { TOKEN_DROPS } == 1);
}

test func forget_does_not_drop_fields() {
  let outer = Outer { inner: Inner { value: 1 } };
  forget(outer);
  assert(unsafe { FIELD_DROPS } == 0);
}

test func forget_through_a_function_value_suppresses_drop() {
  let release: func(ReleaseToken) = forget::<ReleaseToken>;
  release(ReleaseToken { value: 1 });
  assert(unsafe { RELEASE_DROPS } == 0);
}

test func explicit_type_arguments_still_suppress_drop() {
  forget::<ExplicitToken>(ExplicitToken { value: 1 });
  assert(unsafe { EXPLICIT_DROPS } == 0);
}

test func forget_a_generic_value_suppresses_drop() {
  consume(GenericToken { value: 1 });
  assert(unsafe { GENERIC_DROPS } == 0);
}
