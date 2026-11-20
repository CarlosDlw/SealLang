//! Acceptance scenarios for first-class functions, closures, and callable bounds.

use crate::assert_runs;

#[test]
fn function_pointers_flow_through_aggregates_and_branches() {
    assert_runs(
        "function-pointer-aggregates",
        r#"
struct Ops { run: func(i32) -> i32 }

func add_one(value: i32) -> i32 { value + 1 }
func double(value: i32) -> i32 { value * 2 }

func choose(flag: bool) -> func(i32) -> i32 {
  if flag { add_one } else { double }
}

func main() -> i32 {
  let selected = choose(true);
  let ops = Ops { run: selected };
  let pair = (ops.run, 20);
  let array = [double, add_one];
  (pair.0)(pair.1) + array[0](10) + array[1](0)
}
"#,
        42,
    );
}

#[test]
fn a_safe_function_converts_to_an_unsafe_pointer() {
    assert_runs(
        "unsafe-function-pointer",
        r#"
func add_one(value: i32) -> i32 { value + 1 }

func main() -> i32 {
  let callback: unsafe func(i32) -> i32 = add_one;
  unsafe { callback(41) }
}
"#,
        42,
    );
}

#[test]
fn closures_capture_by_reference_and_move_by_value() {
    assert_runs(
        "closure-capture-modes",
        r#"
func main() -> i32 {
  let base = 40;
  let by_ref = || base + 2;

  let mut snapshot = 10;
  let by_value = move || snapshot;
  snapshot = 32;
  by_ref() + by_value() - 10
}
"#,
        42,
    );
}

#[test]
fn fn_fn_mut_and_fn_once_bounds_cover_all_capture_kinds() {
    assert_runs(
        "function-trait-bounds",
        r#"
func twice<F: Fn() -> i32>(f: F) -> i32 { f() + f() }

func each<F: FnMut(i32)>(f: F, count: i32) {
  let mut f = f;
  let mut index = 0;
  while index < count {
    f(index);
    index += 1;
  }
}

func consume<F: FnOnce() -> i32>(f: F) -> i32 { f() }

func main() -> i32 {
  let base = 10;
  let doubled = twice(move || base + 1);

  let mut total = 0;
  each(|value: i32| { total += value; }, 4);

  doubled + total + consume(move || 14)
}
"#,
        42,
    );
}

#[test]
fn a_shared_capture_closure_is_copy() {
    assert_runs(
        "copyable-closure",
        r#"
func main() -> i32 {
  let base = 20;
  let read = || base + 1;
  let copy = read;
  read() + copy()
}
"#,
        42,
    );
}

#[test]
fn a_capture_free_closure_becomes_a_function_pointer() {
    assert_runs(
        "closure-to-function-pointer",
        r#"
func apply<F: Fn(i32) -> i32>(f: F, value: i32) -> i32 {
  f(value)
}

func main() -> i32 {
  let add: func(i32) -> i32 = |value| value + 1;
  add(20) + apply(|value| value * 2, 10) + 1
}
"#,
        42,
    );
}

#[test]
fn an_fn_once_capture_drops_exactly_once() {
    assert_runs(
        "fn-once-drop",
        r#"
static mut DROPS: i32 = 0;

struct Token { value: i32 }

impl Drop for Token {
  func drop(self: *mut Token) {
    unsafe { DROPS += 1; }
  }
}

func main() -> i32 {
  let token = Token { value: 40 };
  let take = move || token;
  let consumed = take();
  drop(consumed);
  unsafe { DROPS + 41 }
}
"#,
        42,
    );
}

#[test]
fn a_closure_environment_drops_its_capture_once_at_scope_exit() {
    assert_runs(
        "closure-environment-drop",
        r#"
static mut DROPS: i32 = 0;

struct Token { value: i32 }

impl Drop for Token {
  func drop(self: *mut Token) {
    unsafe { DROPS += 1; }
  }
}

func make_reader() -> i32 {
  let token = Token { value: 40 };
  let read = move || token.value;
  if read() != 40 { return 1; }
  0
}

func main() -> i32 {
  if make_reader() != 0 { return 1; }
  unsafe { DROPS + 41 }
}
"#,
        42,
    );
}
