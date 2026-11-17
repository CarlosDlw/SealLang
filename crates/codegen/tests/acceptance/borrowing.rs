//! Acceptance scenarios for ownership, moves, and borrowing through generics.

use crate::assert_runs;

#[test]
fn a_generic_argument_is_moved_once_and_returned() {
    assert_runs(
        "move-once",
        r#"
struct Pair<T> { first: T, second: T }

func split<T>(pair: Pair<T>) -> T { pair.first }

func main() -> i32 {
  split(Pair { first: 42, second: 0 })
}
"#,
        42,
    );
}

#[test]
fn a_copy_type_parameter_is_used_twice() {
    assert_runs(
        "copy-twice",
        r#"
func identity<T: Copy>(value: T) -> T { value }

func main() -> i32 {
  let value = 21;
  identity(value) + identity(value)
}
"#,
        42,
    );
}

#[test]
fn a_generic_struct_is_moved_as_a_whole() {
    assert_runs(
        "move-struct",
        r#"
struct Pair<T> { first: T, second: T }

func first_of<T>(pair: Pair<T>) -> T { pair.first }

func main() -> i32 {
  let pair = Pair { first: 42, second: 0 };
  first_of(pair)
}
"#,
        42,
    );
}

#[test]
fn a_generic_field_can_be_moved_out_of_a_partially_used_struct() {
    assert_runs(
        "partial-move",
        r#"
struct Pair<T> { first: T, second: bool }

func main() -> i32 {
  let pair = Pair { first: 42, second: true };
  let first = pair.first;
  first
}
"#,
        42,
    );
}

#[test]
fn a_match_moves_the_generic_payload_out_of_an_enum() {
    assert_runs(
        "enum-payload-move",
        r#"
enum Option<T> { Some(T), None }

struct Pair<T> { first: T, second: T }

func main() -> i32 {
  let value = Option::Some(Pair { first: 42, second: 0 });
  match value { Option::Some(pair) => pair.first, Option::None => 0 }
}
"#,
        42,
    );
}

#[test]
fn pointer_values_are_copied_without_a_copy_bound() {
    assert_runs(
        "pointer-copies",
        r#"
struct Box<T> { value: T }

func shared<T>(value: *Box<T>) -> *Box<T> { value }

func exclusive<T>(value: *mut Box<T>) -> *mut Box<T> { value }

func main() -> i32 {
  let mut boxed = Box { value: 21 };
  let first = shared(&boxed);
  let left = (*first).value;
  let second = exclusive(&mut boxed);
  left + (*second).value
}
"#,
        42,
    );
}

#[test]
fn a_generic_function_returns_a_reference_to_the_caller_frame() {
    assert_runs(
        "return-outer-reference",
        r#"
func identity<T>(value: *T) -> *T { value }

func main() -> i32 {
  let value = 42;
  *identity(&value)
}
"#,
        42,
    );
}

#[test]
fn borrowing_a_generic_field_does_not_block_its_sibling() {
    assert_runs(
        "field-sibling-write",
        r#"
struct Pair<T> { first: T, second: T }

func main() -> i32 {
  let mut pair = Pair { first: 40, second: 1 };
  let reference = &pair.first;
  pair.second = 2;
  *reference + pair.second
}
"#,
        42,
    );
}
