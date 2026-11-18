//! Loans rooted at static and const allocations.

use super::support::{check_clean, check_errors};

#[test]
fn borrowing_a_mutable_static_inside_unsafe_is_allowed() {
    check_clean(
        "static mut COUNTER: i32 = 0;\n\
         func bump() -> i32 {\n  \
           unsafe {\n    let value = &mut COUNTER;\n    *value = *value + 1;\n    *value\n  }\n\
         }\n",
    );
}

#[test]
fn disjoint_fields_of_a_mutable_static_do_not_conflict() {
    check_clean(
        "struct Pair { left: i32, right: i32 }\n\
         static mut PAIR: Pair = Pair { left: 0, right: 0 };\n\
         func write() {\n  \
           unsafe {\n    let left = &mut PAIR.left;\n    let right = &mut PAIR.right;\n    *left = 1;\n    *right = 2;\n  }\n\
         }\n",
    );
}

#[test]
fn a_whole_mutable_static_conflicts_with_a_field_loan() {
    let fixture = check_errors(
        "struct Pair { left: i32, right: i32 }\n\
         static mut PAIR: Pair = Pair { left: 0, right: 0 };\n\
         func write() {\n  \
           unsafe {\n    let field = &mut PAIR.left;\n    let whole = &mut PAIR;\n    *field = 1;\n  }\n\
         }\n",
    );
    fixture.expect("E5008", "already borrowed");
}

#[test]
fn returning_a_mutable_static_reference_is_rejected() {
    let fixture = check_errors(
        "static mut COUNTER: i32 = 0;\n\
         func leak() -> *mut i32 {\n  unsafe { &mut COUNTER }\n}\n",
    );
    fixture.expect("E5012", "mutable static `COUNTER`");
    assert!(
        !fixture
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code().map(|code| code.as_str()) == Some("E5004")),
        "a mutable static must not also be reported as a dangling local"
    );
}

#[test]
fn returning_a_mutable_static_reference_through_a_call_is_rejected() {
    let fixture = check_errors(
        "static mut COUNTER: i32 = 0;\n\
         func identity(value: *mut i32) -> *mut i32 {\n  value\n}\n\
         func leak() -> *mut i32 {\n  unsafe { identity(&mut COUNTER) }\n}\n",
    );
    fixture.expect("E5012", "mutable static `COUNTER`");
}

#[test]
fn returning_a_mutable_static_reference_through_a_field_is_rejected() {
    let fixture = check_errors(
        "struct Holder { value: *mut i32 }\n\
         static mut COUNTER: i32 = 0;\n\
         func leak() -> *mut i32 {\n  \
           unsafe {\n    let holder = Holder { value: &mut COUNTER };\n    holder.value\n  }\n\
         }\n",
    );
    fixture.expect("E5012", "mutable static `COUNTER`");
}

#[test]
fn returning_the_address_of_a_const_field_is_allowed() {
    check_clean(
        "struct Pair { left: i32, right: i32 }\n\
         const PAIR: Pair = Pair { left: 1, right: 2 };\n\
         func address() -> *i32 {\n  &PAIR.left\n}\n",
    );
}

#[test]
fn returning_an_addressed_const_is_allowed() {
    check_clean(
        "const VALUE: i32 = 1;\n\
         func address() -> *i32 {\n  &VALUE\n}\n",
    );
}

#[test]
fn returning_a_local_copy_of_a_const_is_rejected() {
    let fixture = check_errors(
        "const VALUE: i32 = 1;\n\
         func address() -> *i32 {\n  let local = VALUE;\n  &local\n}\n",
    );
    fixture.expect("E5004", "local of this function");
}

#[test]
fn returning_a_shared_static_reference_is_allowed() {
    check_clean(
        "static LIMIT: i32 = 10;\n\
         func address() -> *i32 {\n  &LIMIT\n}\n",
    );
}

#[test]
fn returning_an_exclusive_static_reference_without_returning_it_is_allowed() {
    check_clean(
        "static mut COUNTER: i32 = 0;\n\
         func identity(value: *mut i32) -> *mut i32 {\n  value\n}\n\
         func use_counter() -> i32 {\n  \
           unsafe {\n    let value = identity(&mut COUNTER);\n    *value = 1;\n    *value\n  }\n\
         }\n",
    );
}

#[test]
fn returning_a_const_reference_names_its_static_allocation() {
    check_clean(
        "static BASE: i32 = 5;\n\
         const REF: *i32 = &BASE;\n\
         func address() -> *i32 {\n  REF\n}\n",
    );
}

#[test]
fn returning_a_const_reference_through_a_field_is_allowed() {
    check_clean(
        "struct Pair { left: i32, right: i32 }\n\
         static PAIR: Pair = Pair { left: 1, right: 2 };\n\
         const LEFT: *i32 = &PAIR.left;\n\
         func address() -> *i32 {\n  LEFT\n}\n",
    );
}
