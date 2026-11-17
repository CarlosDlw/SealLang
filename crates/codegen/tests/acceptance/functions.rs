//! Acceptance scenarios for generic functions, inference, and bounds.

use crate::assert_runs;

#[test]
fn identity_accepts_every_supported_argument_shape() {
    assert_runs(
        "identity-shapes",
        r#"
struct Pair<T, U> { first: T, second: U }

func identity<T>(value: T) -> T { value }

func text_score(text: *str) -> i32 { let copy = identity(text); 1 }

func main() -> i32 {
  let number = identity(7);
  let flag = identity(true);
  let text = identity("seal");
  let pair = identity((1, true));
  let array = identity([1, 2, 3]);
  let boxed = identity(Pair { first: 2, second: 3 });
  let flag_score = if flag { 1 } else { 0 };
  number + flag_score + text_score(text) + pair.0 + array[1] + boxed.first + boxed.second
}
"#,
        17,
    );
}

#[test]
fn identity_is_inferred_from_the_expected_return_type() {
    assert_runs(
        "identity-expected",
        r#"
func identity<T>(value: T) -> T { value }

func main() -> i32 {
  let value: i32 = identity(41);
  value + 1
}
"#,
        42,
    );
}

#[test]
fn constructors_accept_explicit_and_inferred_type_arguments() {
    assert_runs(
        "constructor-args",
        r#"
struct Box<T> { value: T }

impl<T> Box<T> {
  func new(value: T) -> Box<T> { Box { value: value } }
}

func main() -> i32 {
  let explicit = Box::<i32>::new(1);
  let inferred: Box<i32> = Box::new(2);
  explicit.value + inferred.value
}
"#,
        3,
    );
}

#[test]
fn two_generic_functions_call_each_other_with_the_same_arguments() {
    assert_runs(
        "mutual-generics",
        r#"
func first_leg<T: Copy>(remaining: i32, value: T) -> T {
  if remaining == 0 { value } else { second_leg(remaining - 1, value) }
}

func second_leg<T: Copy>(remaining: i32, value: T) -> T {
  if remaining == 0 { value } else { first_leg(remaining - 1, value) }
}

func main() -> i32 {
  first_leg(2, 42)
}
"#,
        42,
    );
}

#[test]
fn a_generic_function_transforms_a_type_argument_for_another() {
    assert_runs(
        "transformed-argument",
        r#"
struct Pair<T, U> { first: T, second: U }

func pair_with_true<T>(value: T) -> Pair<T, bool> { Pair { first: value, second: true } }

func wrap<T>(value: T) -> Pair<Pair<T, bool>, i32> {
  Pair { first: pair_with_true(value), second: 1 }
}

func main() -> i32 {
  let wrapped = wrap(40);
  wrapped.first.first + wrapped.second + 1
}
"#,
        42,
    );
}

#[test]
fn a_generic_function_builds_and_returns_aggregates_from_its_parameter() {
    assert_runs(
        "returned-aggregates",
        r#"
struct Pair<T> { first: T, second: T }

enum Option<T> { Some(T), None }

func pair_of<T>(value: T) -> Pair<T> { Pair { first: value, second: value } }

func some_of<T>(value: T) -> Option<T> { Option::Some(value) }

func main() -> i32 {
  let pair = pair_of(3);
  let option = some_of(4);
  let stored = match option { Option::Some(inner) => inner, Option::None => 0 };
  pair.first + pair.second + stored
}
"#,
        10,
    );
}

#[test]
fn three_parameters_can_be_nested_in_another_instantiation() {
    assert_runs(
        "three-parameters",
        r#"
struct Pair<T, U> { first: T, second: U }

func nest<T, U, V>(first: T, second: U, third: V) -> Pair<Pair<T, U>, V> {
  Pair { first: Pair { first: first, second: second }, second: third }
}

func main() -> i32 {
  let nested = nest(1, true, 2);
  nested.first.first + nested.second
}
"#,
        3,
    );
}

#[test]
fn a_where_clause_supplies_the_same_bound_as_an_inline_parameter() {
    assert_runs(
        "where-clause",
        r#"
trait Show { func show(self: *Self) -> i32; }

impl Show for i32 { func show(self: *i32) -> i32 { *self } }

func show_one<T>(value: *T) -> i32 where T: Show { value.show() }

func main() -> i32 {
  let value = 42;
  show_one(&value)
}
"#,
        42,
    );
}

#[test]
fn a_bound_can_apply_to_a_nested_type_argument() {
    assert_runs(
        "nested-bound",
        r#"
struct Pair<T, U> { first: T, second: U }

trait Show { func show(self: *Self) -> i32; }

impl Show for i32 { func show(self: *i32) -> i32 { *self } }

func show_pair<T: Show>(value: *Pair<T, T>) -> i32 {
  (*value).first.show() + (*value).second.show()
}

func main() -> i32 {
  let pair = Pair { first: 40, second: 2 };
  show_pair(&pair)
}
"#,
        42,
    );
}
