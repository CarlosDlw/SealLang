//! Acceptance scenarios for inherent methods and associated functions.

use crate::assert_runs;

#[test]
fn an_inherent_method_returns_the_parameter_type() {
    assert_runs(
        "method-returns-param",
        r#"
struct Box<T> { value: T }

impl<T> Box<T> {
  func read(self: *Box<T>) -> T { (*self).value }
}

func main() -> i32 {
  let boxed = Box { value: 42 };
  boxed.read()
}
"#,
        42,
    );
}

#[test]
fn shared_and_exclusive_receivers_share_one_generic_type() {
    assert_runs(
        "method-receivers",
        r#"
struct Box<T> { value: T }

impl<T> Box<T> {
  func read(self: *Box<T>) -> T { (*self).value }
  func write(self: *mut Box<T>, value: T) { (*self).value = value; }
}

func main() -> i32 {
  let mut boxed = Box { value: 1 };
  boxed.write(42);
  boxed.read()
}
"#,
        42,
    );
}

#[test]
fn an_associated_constructor_infers_all_type_arguments() {
    assert_runs(
        "method-constructor",
        r#"
struct Box<T> { value: T }

impl<T> Box<T> {
  func new(value: T) -> Box<T> { Box { value: value } }
}

func main() -> i32 {
  let boxed = Box::new(42);
  boxed.value
}
"#,
        42,
    );
}

#[test]
fn a_generic_enum_has_an_inherent_method_and_a_constructor() {
    assert_runs(
        "enum-methods",
        r#"
enum Option<T> { Some(T), None }

impl<T> Option<T> {
  func is_some(self: *Option<T>) -> bool {
    match self { Option::Some(_) => true, Option::None => false }
  }
  func some(value: T) -> Option<T> { Option::Some(value) }
}

func main() -> i32 {
  let option = Option::some(42);
  if option.is_some() { 42 } else { 0 }
}
"#,
        42,
    );
}

#[test]
fn concrete_impls_resolve_only_for_their_type_arguments() {
    assert_runs(
        "concrete-impls",
        r#"
struct Pair<T, U> { first: T, second: U }

impl Pair<i32, i32> {
  func sum(self: *Pair<i32, i32>) -> i32 { (*self).first + (*self).second }
}

impl Pair<bool, bool> {
  func both(self: *Pair<bool, bool>) -> bool { (*self).first && (*self).second }
}

func main() -> i32 {
  let numbers = Pair { first: 40, second: 2 };
  let flags = Pair { first: true, second: true };
  if flags.both() { numbers.sum() } else { 0 }
}
"#,
        42,
    );
}

#[test]
fn a_generic_impl_method_has_its_own_parameter_and_turbofish() {
    assert_runs(
        "method-own-param",
        r#"
struct Box<T> { value: T }

impl<T> Box<T> {
  func replace<U>(self: *mut Box<T>, value: U) -> U { value }
}

func main() -> i32 {
  let mut boxed = Box { value: 42 };
  let replaced = boxed.replace::<bool>(true);
  if replaced { boxed.value } else { 0 }
}
"#,
        42,
    );
}

#[test]
fn an_exclusive_receiver_reborrows_shared_for_a_read_only_method() {
    assert_runs(
        "method-reborrow",
        r#"
struct Box<T> { value: T }

impl<T> Box<T> {
  func read(self: *Box<T>) -> T { (*self).value }
  func read_through_mut(self: *mut Box<T>) -> T { self.read() }
}

func main() -> i32 {
  let mut boxed = Box { value: 42 };
  boxed.read_through_mut()
}
"#,
        42,
    );
}

#[test]
fn a_method_calls_another_method_on_the_same_generic_type() {
    assert_runs(
        "method-calls-method",
        r#"
struct Box<T> { value: T }

impl<T> Box<T> {
  func read(self: *Box<T>) -> T { (*self).value }
  func read_again(self: *Box<T>) -> T { self.read() }
}

func main() -> i32 {
  let boxed = Box { value: 42 };
  boxed.read_again()
}
"#,
        42,
    );
}

#[test]
fn a_method_on_a_bound_type_parameter_calls_the_bound_method() {
    assert_runs(
        "method-bound-call",
        r#"
trait Show { func show(self: *Self) -> i32; }

struct Wrapper<T> { value: T }

impl<T: Show> Show for Wrapper<T> {
  func show(self: *Wrapper<T>) -> i32 { (*self).value.show() }
}

impl Show for i32 {
  func show(self: *i32) -> i32 { *self }
}

func read<T: Show>(value: *T) -> i32 { value.show() }

func main() -> i32 {
  let wrapper = Wrapper { value: 42 };
  read(&wrapper)
}
"#,
        42,
    );
}
