//! Acceptance scenarios for generic structs, enums, and patterns.

use crate::assert_runs;

#[test]
fn a_two_parameter_struct_is_built_by_inference_and_explicit_arguments() {
    assert_runs(
        "struct-two-params",
        r#"
struct Pair<T, U> { first: T, second: U }

impl<T, U> Pair<T, U> {
  func combine(first: T, second: U) -> Pair<T, U> {
    Pair { first: first, second: second }
  }
}

func main() -> i32 {
  let explicit = Pair::<i32, bool>::combine(40, true);
  let inferred: Pair<i32, i32> = Pair { first: 1, second: 1 };
  explicit.first + inferred.first + inferred.second
}
"#,
        42,
    );
}

#[test]
fn a_zero_field_struct_accepts_an_annotation() {
    assert_runs(
        "struct-zero-fields",
        r#"
struct Marker<T> { }

func main() -> i32 {
  let marker: Marker<i32> = Marker { };
  42
}
"#,
        42,
    );
}

#[test]
fn a_struct_can_store_a_value_and_a_reference_to_one() {
    assert_runs(
        "struct-reference-field",
        r#"
struct Link<T> { value: T, next: *T }

func main() -> i32 {
  let tail = 2;
  let link = Link { value: 40, next: &tail };
  link.value + *link.next
}
"#,
        42,
    );
}

#[test]
fn a_struct_can_store_arrays_and_tuples_of_its_parameter() {
    assert_runs(
        "struct-array-tuple-fields",
        r#"
struct Mixed<T> { values: [T; 2], pair: (T, T) }

func main() -> i32 {
  let mixed = Mixed { values: [1, 2], pair: (3, 4) };
  mixed.values[0] + mixed.pair.1 + mixed.values[1] + mixed.pair.0 + 32
}
"#,
        42,
    );
}

#[test]
fn an_enum_supports_unit_positional_and_named_variants() {
    assert_runs(
        "enum-variants",
        r#"
enum Message<T> { Empty, Value(T), Named { value: T } }

func main() -> i32 {
  let empty: Message<i32> = Message::Empty;
  let positional = Message::Value(40);
  let named = Message::Named { value: 2 };
  let a = match empty { Message::Empty => 0, Message::Value(value) => value, Message::Named { value } => value };
  let b = match positional { Message::Empty => 0, Message::Value(value) => value, Message::Named { value } => value };
  let c = match named { Message::Empty => 0, Message::Value(value) => value, Message::Named { value } => value };
  a + b + c
}
"#,
        42,
    );
}

#[test]
fn a_two_parameter_enum_uses_both_parameters_in_one_variant() {
    assert_runs(
        "enum-two-params",
        r#"
enum Either<T, U> { Left(T), Right(U), Both { left: T, right: U } }

func main() -> i32 {
  let value = Either::Both { left: 40, right: true };
  match value {
    Either::Left(left) => left,
    Either::Right(right) => 0,
    Either::Both { left, right } => left + 2,
  }
}
"#,
        42,
    );
}

#[test]
fn a_recursive_generic_enum_traverses_through_references() {
    assert_runs(
        "recursive-enum",
        r#"
enum List<T> { Empty, Node(T, *List<T>) }

func sum(list: *List<i32>) -> i32 {
  match list { List::Node(value, next) => value + sum(next), List::Empty => 0 }
}

func main() -> i32 {
  let tail: List<i32> = List::Empty;
  let middle = List::Node(2, &tail);
  let head = List::Node(40, &middle);
  sum(&head)
}
"#,
        42,
    );
}

#[test]
fn a_recursive_generic_struct_traverses_through_references() {
    assert_runs(
        "recursive-struct",
        r#"
enum Option<T> { Some(T), None }

struct Node<T> { value: T, next: Option<*Node<T>> }

func sum(node: *Node<i32>) -> i32 {
  let value = (*node).value;
  let next = match &(*node).next {
    Option::Some(pointer) => sum(pointer),
    Option::None => 0,
  };
  value + next
}

func main() -> i32 {
  let tail = Node { value: 2, next: Option::None };
  let head = Node { value: 40, next: Option::Some(&tail) };
  sum(&head)
}
"#,
        42,
    );
}

#[test]
fn a_match_covers_a_generic_enum_of_a_generic_struct() {
    assert_runs(
        "nested-match",
        r#"
enum Option<T> { Some(T), None }

struct Pair<T, U> { first: T, second: U }

func main() -> i32 {
  let value = Option::Some(Pair { first: 40, second: true });
  match value { Option::Some(pair) => pair.first + 2, Option::None => 0 }
}
"#,
        42,
    );
}

#[test]
fn a_generic_function_matches_its_own_option() {
    assert_runs(
        "generic-match",
        r#"
enum Option<T> { Some(T), None }

func unwrap_or<T: Copy>(value: Option<T>, fallback: T) -> T {
  match value { Option::Some(inner) => inner, Option::None => fallback }
}

func main() -> i32 {
  let present = Option::Some(40);
  let missing: Option<i32> = Option::None;
  unwrap_or(present, 0) + unwrap_or(missing, 2)
}
"#,
        42,
    );
}

#[test]
fn a_for_loop_iterates_a_generic_array() {
    assert_runs(
        "generic-for",
        r#"
func count<T: Copy>(values: [T; 2]) -> usize {
  let mut total: usize = 0;
  for value in values { total = total + 1; }
  total
}

func main() -> i32 {
  if count([40, 2]) == 2 { 42 } else { 0 }
}
"#,
        42,
    );
}

#[test]
fn a_generic_function_accepts_a_slice_reference() {
    assert_runs(
        "generic-slice",
        r#"
func slice_len<T>(values: *[T]) -> usize { values.len() }

func main() -> i32 {
  let values = [1, 2, 3];
  let slice = &values[0..3];
  if slice_len(slice) == 3 && values[2] == 3 { 42 } else { 0 }
}
"#,
        42,
    );
}

#[test]
fn a_generic_tuple_is_returned_and_destructured() {
    assert_runs(
        "generic-tuple",
        r#"
struct Pair<T, U> { first: T, second: U }

func first_of<T, U>(pair: Pair<T, U>) -> T { pair.first }

func main() -> i32 {
  let pair = Pair { first: 40, second: true };
  let total = match (pair.first, pair.second) { (left, right) => if right { 2 } else { 0 } };
  first_of(pair) + total
}
"#,
        42,
    );
}

#[test]
fn a_generic_array_of_structs_is_indexed_and_matched() {
    assert_runs(
        "generic-struct-array",
        r#"
struct Pair<T, U> { first: T, second: U }

func second<T: Copy, U: Copy>(values: [Pair<T, U>; 2]) -> U {
  let first = values[0].first;
  match values[1] { Pair { second, .. } => second }
}

func main() -> i32 {
  let values = [
    Pair { first: 1, second: 2 },
    Pair { first: 3, second: 40 },
  ];
  second(values) + 2
}
"#,
        42,
    );
}
