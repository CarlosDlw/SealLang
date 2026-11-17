//! Command-line regression fixtures for generic diagnostics.
//!
//! Each case asserts the stable code, a message fragment, and exit status 1 for
//! a program a user can write. Internal-limit diagnostics that need lowered
//! collection limits stay covered by the monomorphization unit tests.

mod support;

use support::reject;

#[test]
fn a_method_declared_by_two_traits_is_ambiguous() {
    reject(
        "generic-method-ambiguous",
        r#"
trait Left { func name(self: *Self) -> i32; }
trait Right { func name(self: *Self) -> i32; }
struct Value { field: i32 }
impl Left for Value { func name(self: *Value) -> i32 { (*self).field } }
impl Right for Value { func name(self: *Value) -> i32 { (*self).field + 1 } }
func main() -> i32 { let value = Value { field: 1 }; value.name() }
"#,
        "E2028",
        "declared by more than one trait",
    );
}

#[test]
fn a_zero_field_struct_without_an_annotation_is_reported() {
    reject(
        "generic-zero-field",
        r#"
struct Marker<T> { }
func main() -> i32 { let marker = Marker { }; 0 }
"#,
        "E3038",
        "cannot infer type parameter",
    );
}

#[test]
fn a_missing_implementation_is_reported() {
    reject(
        "generic-missing-impl",
        r#"
trait Show { func show(self: *Self) -> i32; }
func show_one<T: Show>(value: *T) -> i32 { value.show() }
func main() -> i32 { let value = 1; show_one(&value) }
"#,
        "E3039",
        "not satisfied",
    );
}

#[test]
fn a_conditional_impl_does_not_apply_to_a_non_copy_type() {
    reject(
        "generic-conditional-impl",
        r#"
struct Token { value: *i32 }
struct Pair<T> { first: T, second: T }
trait Mark { func mark(self: *Self) -> i32; }
impl<T: Copy> Mark for Pair<T> { func mark(self: *Pair<T>) -> i32 { 1 } }
func apply<T: Mark>(value: *T) -> i32 { value.mark() }
func main() -> i32 {
  let number = 1;
  let first = Token { value: &number };
  let second = Token { value: &number };
  let pair = Pair { first: first, second: second };
  apply(&pair)
}
"#,
        "E3039",
        "not satisfied",
    );
}

#[test]
fn a_non_copy_argument_does_not_satisfy_a_copy_bound() {
    reject(
        "generic-copy-bound",
        r#"
struct Token { value: *i32 }
func duplicate<T: Copy>(value: T) -> T { value }
func main() -> i32 {
  let number = 1;
  let token = Token { value: &number };
  duplicate(token);
  0
}
"#,
        "E3043",
        "does not satisfy the `Copy` bound",
    );
}

#[test]
fn a_method_type_argument_list_must_match_its_parameters() {
    reject(
        "generic-method-type-args",
        r#"
struct Box<T> { value: T }
impl<T> Box<T> {
  func set<U>(self: *mut Box<T>, value: U) { }
}
func main() -> i32 {
  let mut boxed = Box { value: 1 };
  boxed.set::<i32, bool>(1);
  0
}
"#,
        "E3044",
        "takes 1 type argument",
    );
}

#[test]
fn two_bounds_with_the_same_method_are_ambiguous() {
    reject(
        "generic-ambiguous-bounds",
        r#"
trait Left { func mark(self: *Self) -> i32; }
trait Right { func mark(self: *Self) -> i32; }
func apply<T: Left + Right>(value: *T) -> i32 { value.mark() }
func main() -> i32 { 0 }
"#,
        "E3040",
        "more than one bound",
    );
}

#[test]
fn arithmetic_on_an_unbounded_parameter_is_rejected() {
    reject(
        "generic-parameter-arithmetic",
        r#"
func add<T>(left: T, right: T) -> T { left + right }
func main() -> i32 { add(1, 2) }
"#,
        "E3041",
        "not available for the type parameter",
    );
}

#[test]
fn a_self_referential_bound_reports_the_recursion_limit() {
    reject(
        "generic-bound-cycle",
        r#"
trait Loop { }
impl<T: Loop> Loop for T { }
func need<T: Loop>(value: T) { }
func main() { need(1); }
"#,
        "E3042",
        "recursion limit",
    );
}

#[test]
fn growing_generic_recursion_reports_e7001() {
    reject(
        "generic-growing-recursion",
        r#"
func grow<T>(value: T) -> i32 { grow((value,)) }
func main() -> i32 { grow(1) }
"#,
        "E7001",
        "recursion limit",
    );
}

#[test]
fn a_generic_entry_point_is_rejected() {
    reject(
        "generic-entry-point",
        "func main<T>() -> i32 { 0 }",
        "E7003",
        "entry point cannot be generic",
    );
}

#[test]
fn returning_a_reference_to_a_generic_local_is_rejected() {
    reject(
        "generic-local-reference",
        r#"
func local_ref<T>(value: T) -> *T { &value }
func main() -> i32 { let value = 1; *local_ref(value) }
"#,
        "E5004",
        "cannot return a reference to",
    );
}

#[test]
fn a_non_copy_generic_argument_cannot_be_used_twice() {
    reject(
        "generic-move-twice",
        r#"
struct Pair<T> { first: T, second: T }
func consume<T>(pair: Pair<T>) -> i32 { 0 }
func main() -> i32 {
  let pair = Pair { first: 1, second: 2 };
  consume(pair);
  consume(pair)
}
"#,
        "E5005",
        "was moved",
    );
}

#[test]
fn a_generic_aggregate_that_may_hide_a_reference_keeps_its_loan() {
    reject(
        "generic-hidden-loan",
        r#"
struct Holder<T> { value: T }
func keep<T>(value: T) -> Holder<T> { Holder { value: value } }
func main() -> i32 {
  let mut number = 1;
  let holder = keep(&number);
  number = 2;
  (*holder.value) + number
}
"#,
        "E5001",
        "because it is borrowed",
    );
}

#[test]
fn a_refutable_for_pattern_is_rejected() {
    reject(
        "for-refutable",
        "func main() -> i32 { for 0 in 0..3 { } 0 }",
        "E3034",
        "does not match every element",
    );
}

#[test]
fn a_non_iterable_for_source_is_rejected() {
    reject(
        "for-not-iterable",
        "func main() -> i32 { for value in 5 { } 0 }",
        "E3035",
        "cannot iterate over",
    );
}

#[test]
fn a_tuple_index_outside_the_arity_is_rejected() {
    reject(
        "tuple-index",
        "func main() -> i32 { let pair = (1, true); pair.2 }",
        "E3036",
        "out of bounds",
    );
}

#[test]
fn a_tuple_pattern_with_the_wrong_arity_is_rejected() {
    reject(
        "tuple-pattern-arity",
        "func main(pair: (i32, i32)) -> i32 { match pair { (a, b, c) => a } }",
        "E3037",
        "3 element(s)",
    );
}
