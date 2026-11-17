//! Acceptance scenarios for generic traits and implementations.

use crate::assert_runs;

#[test]
fn a_one_parameter_trait_exposes_one_method() {
    assert_runs(
        "trait-one-param",
        r#"
trait Value<T> { func value(self: *Self) -> T; }

impl Value<i32> for bool {
  func value(self: *bool) -> i32 { if *self { 42 } else { 0 } }
}

func main() -> i32 {
  let value: i32 = true.value();
  value
}
"#,
        42,
    );
}

#[test]
fn a_two_parameter_trait_uses_both_parameters_in_one_method() {
    assert_runs(
        "trait-two-params",
        r#"
trait Combine<A, B> { func combine(self: *Self, first: *A, second: *B) -> i32; }

impl Combine<i32, bool> for i32 {
  func combine(self: *i32, first: *i32, second: *bool) -> i32 {
    (*self) + (*first) + (if *second { 40 } else { 0 })
  }
}

func main() -> i32 {
  let one = 1;
  let flag = true;
  1.combine(&one, &flag)
}
"#,
        42,
    );
}

#[test]
fn a_trait_method_has_its_own_parameter_and_return() {
    assert_runs(
        "trait-own-param",
        r#"
trait Tag { func tag<U: Copy>(self: *Self, value: U) -> U; }

impl Tag for i32 {
  func tag<U: Copy>(self: *i32, value: U) -> U { value }
}

func main() -> i32 {
  1.tag::<i32>(42)
}
"#,
        42,
    );
}

#[test]
fn a_trait_method_can_return_self() {
    assert_runs(
        "trait-return-self",
        r#"
trait Duplicate { func duplicate(self: *Self) -> Self; }

struct Holder { stored: i32 }

impl Duplicate for Holder {
  func duplicate(self: *Holder) -> Holder { Holder { stored: (*self).stored } }
}

func copy_of<T: Duplicate>(value: *T) -> T { value.duplicate() }

func main() -> i32 {
  let holder = Holder { stored: 42 };
  let copy = copy_of(&holder);
  copy.stored
}
"#,
        42,
    );
}

#[test]
fn a_trait_method_can_return_its_trait_argument() {
    assert_runs(
        "trait-return-argument",
        r#"
trait Convert<T> { func convert(self: *Self) -> T; }

impl Convert<bool> for i32 {
  func convert(self: *i32) -> bool { *self == 42 }
}

func main() -> i32 {
  let converted: bool = 42.convert();
  if converted { 42 } else { 0 }
}
"#,
        42,
    );
}

#[test]
fn a_generic_trait_is_implemented_for_a_generic_struct() {
    assert_runs(
        "trait-generic-struct",
        r#"
trait Scale { func scale(self: *Self) -> i32; }

struct Wrapper<T> { value: T }

impl<T: Scale> Scale for Wrapper<T> {
  func scale(self: *Wrapper<T>) -> i32 { (*self).value.scale() }
}

impl Scale for i32 {
  func scale(self: *i32) -> i32 { *self }
}

func main() -> i32 {
  let wrapper = Wrapper { value: 42 };
  wrapper.scale()
}
"#,
        42,
    );
}

#[test]
fn a_generic_trait_is_implemented_for_a_generic_enum() {
    assert_runs(
        "trait-generic-enum",
        r#"
enum Option<T> { Some(T), None }

trait Value { func value(self: *Self) -> i32; }

impl<T: Value + Copy> Value for Option<T> {
  func value(self: *Option<T>) -> i32 {
    match self { Option::Some(inner) => inner.value(), Option::None => 0 }
  }
}

impl Value for i32 {
  func value(self: *i32) -> i32 { *self }
}

func main() -> i32 {
  let option = Option::Some(42);
  option.value()
}
"#,
        42,
    );
}

#[test]
fn a_trait_is_implemented_for_primitives_arrays_tuples_and_references() {
    assert_runs(
        "trait-aggregate-impls",
        r#"
trait Tag { func tag(self: *Self) -> i32; }

impl Tag for i32 { func tag(self: *i32) -> i32 { *self } }

impl Tag for [i32; 2] {
  func tag(self: *[i32; 2]) -> i32 { (*self)[0] + (*self)[1] }
}

impl Tag for (i32, i32) {
  func tag(self: *(i32, i32)) -> i32 { (*self).0 + (*self).1 }
}

impl Tag for *i32 {
  func tag(self: **i32) -> i32 { **self }
}

func read<T: Tag>(value: *T) -> i32 { value.tag() }

func main() -> i32 {
  let array = [10, 11];
  let tuple = (10, 11);
  let number = 1;
  let pointer = &number;
  read(&array) - 20 + read(&tuple) - 20 + read(&pointer) + 39
}
"#,
        42,
    );
}

#[test]
fn a_blanket_impl_provides_a_trait_for_every_bounded_type() {
    assert_runs(
        "trait-blanket",
        r#"
trait Base { func base(self: *Self) -> i32; }

trait Mark { func mark(self: *Self) -> i32; }

impl Base for i32 {
  func base(self: *i32) -> i32 { *self }
}

impl<T: Base> Mark for T {
  func mark(self: *T) -> i32 { self.base() + 2 }
}

func read<T: Mark>(value: *T) -> i32 { value.mark() }

func main() -> i32 {
  let number = 40;
  read(&number)
}
"#,
        42,
    );
}

#[test]
fn a_conditional_copy_impl_is_selected_for_copy_arguments() {
    assert_runs(
        "trait-conditional-copy",
        r#"
struct Pair<T> { first: T, second: T }

trait Mark { func mark(self: *Self) -> i32; }

impl<T: Copy> Mark for Pair<T> {
  func mark(self: *Pair<T>) -> i32 { 42 }
}

func read<T: Mark>(value: *T) -> i32 { value.mark() }

func main() -> i32 {
  let pair = Pair { first: 1, second: 2 };
  read(&pair)
}
"#,
        42,
    );
}

#[test]
fn two_traits_with_different_methods_share_a_type() {
    assert_runs(
        "trait-different-methods",
        r#"
trait Show { func show(self: *Self) -> i32; }

trait Tag { func tag(self: *Self) -> i32; }

impl Show for i32 { func show(self: *i32) -> i32 { *self } }

impl Tag for i32 { func tag(self: *i32) -> i32 { *self + 2 } }

func main() -> i32 {
  40.show() + 0.tag()
}
"#,
        42,
    );
}

#[test]
fn two_traits_with_the_same_method_name_coexist_until_a_call() {
    assert_runs(
        "trait-same-name-unused",
        r#"
trait Left { func name(self: *Self) -> i32; }

trait Right { func name(self: *Self) -> i32; }

struct Value { field: i32 }

impl Left for Value { func name(self: *Value) -> i32 { (*self).field } }

impl Right for Value { func name(self: *Value) -> i32 { (*self).field + 1 } }

func main() -> i32 { 42 }
"#,
        42,
    );
}

#[test]
fn a_subtrait_bound_reaches_the_supertrait_method() {
    assert_runs(
        "trait-supertrait",
        r#"
trait Root { func root(self: *Self) -> i32; }

trait Leaf: Root { func leaf(self: *Self) -> i32; }

impl Root for i32 { func root(self: *i32) -> i32 { *self } }

impl Leaf for i32 {
  func leaf(self: *i32) -> i32 { self.root() + 2 }
}

func read<T: Leaf>(value: *T) -> i32 { value.leaf() + value.root() }

func main() -> i32 {
  let value = 20;
  read(&value)
}
"#,
        42,
    );
}

#[test]
fn a_trait_method_calls_a_generic_method_on_a_bound_type() {
    assert_runs(
        "trait-calls-bound-generic",
        r#"
trait Render { func render(self: *Self) -> i32; }

trait Apply { func apply<R: Render>(self: *Self, value: *R) -> i32; }

struct Point { value: i32 }

impl Render for Point {
  func render(self: *Point) -> i32 { (*self).value }
}

impl Apply for i32 {
  func apply<R: Render>(self: *i32, value: *R) -> i32 { (*self) + value.render() }
}

func main() -> i32 {
  let point = Point { value: 40 };
  2.apply(&point)
}
"#,
        42,
    );
}
