//! Borrow checking of generic bodies.

use seal_def::DefId;
use seal_hir::Hir;

use super::support::{Fixture, check_clean, check_errors};

/// Returns the definition of the function named `name`.
fn function_owner(fixture: &Fixture, hir: &Hir, name: &str) -> DefId {
    hir.bodies()
        .iter()
        .map(|body| body.owner)
        .find(|owner| fixture.symbol(hir.definitions().name(*owner)) == name)
        .unwrap_or_else(|| panic!("no function named `{name}`"))
}

#[test]
fn a_copy_bound_parameter_can_be_reused() {
    check_clean(
        "func duplicate<T: Copy>(value: T) -> T {\n  let first = value;\n  let second = value;\n  first\n}\n\n\
         func main() -> i32 { duplicate(7) }\n",
    );
}

#[test]
fn moving_a_parameter_consumes_it() {
    let fixture = check_errors(
        "func consume<T>(value: T) -> T {\n  let first = value;\n  let second = value;\n  first\n}\n\n\
         func main() -> i32 { consume(7) }\n",
    );
    fixture.expect("E5005", "moved");
}

#[test]
fn returning_a_reference_to_a_parameter_local_is_rejected() {
    let fixture = check_errors("func leak<T>(value: T) -> *T {\n  &value\n}\n");
    fixture.expect("E5004", "local");
}

#[test]
fn a_generic_aggregate_tracks_partial_moves_per_field() {
    check_clean(
        "struct Pair<T> { first: T, second: T }\n\n\
         func steal<T>(pair: Pair<T>) -> T {\n  let moved = pair.first;\n  pair.second\n}\n",
    );
}

#[test]
fn a_generic_tuple_keeps_siblings_usable() {
    check_clean(
        "func tuple_first<T>(left: T, right: T) -> T {\n  let items = (left, right);\n  items.0\n}\n",
    );
}

#[test]
fn a_generic_array_element_move_needs_copy() {
    let fixture = check_errors(
        "func array_first<T>(left: T, right: T) -> T {\n  let items = [left, right];\n  items[0]\n}\n",
    );
    fixture.expect("E5015", "element");
}

#[test]
fn a_copy_array_element_move_is_allowed() {
    check_clean(
        "func array_first<T: Copy>(left: T, right: T) -> T {\n  let items = [left, right];\n  items[0]\n}\n",
    );
}

#[test]
fn a_moved_generic_receiver_cannot_be_used_again() {
    let fixture = check_errors(
        "struct Wrapper<T> { value: T }\n\n\
         impl<T> Wrapper<T> {\n  func take(self: Wrapper<T>) -> T { self.value }\n}\n\n\
         func main() -> i32 {\n  let wrapper = Wrapper { value: 1 };\n  \
         let first = wrapper.take();\n  let second = wrapper.take();\n  first\n}\n",
    );
    fixture.expect("E5005", "moved");
}

#[test]
fn a_symbolic_call_rejects_aliasing_exclusive_arguments() {
    let fixture = check_errors(
        "trait Merge { func merge(self: *mut Self, other: *mut Self); }\n\n\
         func run<T: Merge>(value: *mut T) {\n  let alias = value;\n  value.merge(alias);\n}\n\n\
         func main() -> i32 { 0 }\n",
    );
    fixture.expect("E5010", "argument");
}

#[test]
fn a_generic_aggregate_carries_the_loans_of_its_instantiated_fields() {
    let fixture = check_errors(
        "struct Holder<T> { value: T }\n\n\
         func wrap<T>(value: *mut T) -> Holder<*mut T> {\n  Holder { value: value }\n}\n\n\
         func main() -> i32 {\n  let mut x = 1;\n  let holder = wrap(&mut x);\n  \
         let value = x;\n  let pointer = holder.value;\n  value\n}\n",
    );
    fixture.expect("E5007", "exclusively borrowed");
}

#[test]
fn noalias_is_withheld_from_a_generic_exclusive_parameter() {
    let fixture = Fixture::new(
        "func bump<T>(value: *mut T) {\n}\n\n\
         func clear(value: *mut i32) {\n  *value = 0;\n}\n\n\
         func main() -> i32 {\n  let mut x = 1;\n  bump(&mut x);\n  clear(&mut x);\n  0\n}\n",
    );
    let (hir, _mir, facts) = fixture.check_facts();
    let bump = function_owner(&fixture, &hir, "bump");
    let clear = function_owner(&fixture, &hir, "clear");
    assert!(
        !facts.noalias_functions().contains(&bump),
        "a parameter whose pointee changes per instantiation is not proven"
    );
    assert!(
        facts.noalias_functions().contains(&clear),
        "a concrete exclusive parameter keeps its proof"
    );
}

#[test]
fn a_generic_reference_argument_is_checked_with_the_same_rules() {
    check_clean(
        "trait Peek { func peek(self: *Self) -> i32; }\n\n\
         func peek_one<T: Peek>(value: *T) -> i32 { value.peek() }\n\n\
         func main() -> i32 { 0 }\n",
    );
}
