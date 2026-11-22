// Forgetting a value must suppress every destructor reached through its
// fields, enum payloads, tuple elements, array elements, and closure
// environment. Each test uses a distinct destructor counter.

use std::mem::forget;

struct Leaf {
  value: i32,
}

struct Pair {
  left: Leaf,
  right: Leaf,
}

struct Nested {
  pair: Pair,
  leaf: Leaf,
}

enum Choice {
  One(Leaf),
  Two { leaf: Leaf },
}

struct FieldToken {
  value: i32,
}

struct FieldHolder {
  token: FieldToken,
  count: i32,
}

struct ViewHolder {
  value: *i32,
}

struct Empty {}

struct ConstToken {
  value: i32,
}

struct ClosureToken {
  value: i32,
}

static mut LEAF_DROPS: i32 = 0;
static mut FIELD_DROPS: i32 = 0;
static mut VIEW_DROPS: i32 = 0;
static mut EMPTY_DROPS: i32 = 0;
static mut CONST_DROPS: i32 = 0;
static mut CLOSURE_DROPS: i32 = 0;

impl Drop for Leaf {
  func drop(self: *mut Leaf) {
    unsafe {
      LEAF_DROPS += 1;
    }
  }
}

impl Drop for FieldToken {
  func drop(self: *mut FieldToken) {
    unsafe {
      FIELD_DROPS += 1;
    }
  }
}

impl Drop for Empty {
  func drop(self: *mut Empty) {
    unsafe {
      EMPTY_DROPS += 1;
    }
  }
}

impl Drop for ConstToken {
  func drop(self: *mut ConstToken) {
    unsafe {
      CONST_DROPS += 1;
    }
  }
}

impl Drop for ClosureToken {
  func drop(self: *mut ClosureToken) {
    unsafe {
      CLOSURE_DROPS += 1;
    }
  }
}

impl Drop for ViewHolder {
  func drop(self: *mut ViewHolder) {
    unsafe {
      VIEW_DROPS += 1;
    }
  }
}

const LEAKED: ConstToken = ConstToken { value: 1 };
static COPY_STATIC: i32 = 5;

func forget_tuple_and_array() {
  let tuple = (Leaf { value: 1 }, Leaf { value: 2 });
  forget(tuple);

  let array: [Leaf; 2] = [Leaf { value: 3 }, Leaf { value: 4 }];
  forget(array);

  let empty: [Leaf; 0] = [];
  forget(empty);
}

func forget_enum() {
  forget(Choice::One(Leaf { value: 5 }));
  forget(Choice::Two { leaf: Leaf { value: 6 } });
}

func forget_nested() {
  forget(Nested {
    pair: Pair {
      left: Leaf { value: 7 },
      right: Leaf { value: 8 },
    },
    leaf: Leaf { value: 9 },
  });
}

func forget_closure_capture() {
  let token = ClosureToken { value: 10 };
  let release = move || {
    forget(token);
  };
  release();
}

test func forgetting_aggregates_suppresses_every_field() {
  forget_tuple_and_array();
  forget_enum();
  forget_nested();
  assert(unsafe { LEAF_DROPS } == 0);
}

test func forgetting_a_field_leaves_the_sibling_usable() {
  let holder = FieldHolder {
    token: FieldToken { value: 11 },
    count: 12,
  };
  forget(holder.token);
  assert(unsafe { FIELD_DROPS } == 0);
  assert(holder.count == 12);
}

test func forgetting_a_constant_materializes_without_drop() {
  forget(LEAKED);
  assert(unsafe { CONST_DROPS } == 0);
}

test func forgetting_a_copy_static_reads_it_without_moving() {
  forget(COPY_STATIC);
  assert(COPY_STATIC == 5);
}

test func forgetting_a_zero_sized_value_suppresses_drop() {
  forget(Empty {});
  assert(unsafe { EMPTY_DROPS } == 0);
}

test func forgetting_a_holder_releases_its_embedded_view() {
  let mut number = 1;
  let holder = ViewHolder { value: &number };
  forget(holder);
  assert(unsafe { VIEW_DROPS } == 0);
  number = 2;
  assert(number == 2);
}

test func forgetting_a_copy_view_does_not_move_it() {
  let number = 1;
  let view = &number;
  forget(view);
  assert(*view == 1);
}

test func forgetting_a_view_ends_its_last_use() {
  let mut number = 1;
  let view = &number;
  forget(view);
  number = 2;
  assert(number == 2);
}

test func forgetting_a_copy_dereference_reads_without_moving() {
  let mut number = 1;
  let shared = &number;
  forget(*shared);
  assert(*shared == 1);

  let exclusive: *mut i32 = &mut number;
  forget(*exclusive);
  assert(number == 1);
}

test func forgetting_a_closure_capture_does_not_drop_it_with_the_closure() {
  forget_closure_capture();
  assert(unsafe { CLOSURE_DROPS } == 0);
}

func keep_copy<T: Copy>(value: T) {
  forget(value);
}

test func a_copy_bounded_generic_remains_usable() {
  let number = 13;
  keep_copy(number);
  assert(number == 13);
}
