//! Escaping references.

use super::support::{check_clean, check_errors};

#[test]
fn returning_a_local_reference_is_rejected() {
    check_errors("func f() -> *i32 {\n  let x = 1;\n  &x\n}\n").expect("E5004", "local");
}

#[test]
fn returning_a_reference_stored_in_a_local_is_rejected() {
    let fixture = check_errors("func f() -> *i32 {\n  let x = 1;\n  let r = &x;\n  r\n}\n");
    fixture.expect("E5004", "local");
}

#[test]
fn returning_a_parameter_reference_is_allowed() {
    check_clean("func f(p: *i32) -> *i32 {\n  p\n}\n");
}

#[test]
fn returning_a_reference_through_a_parameter_is_allowed() {
    check_clean("func f(p: *i32) -> *i32 {\n  &*p\n}\n");
}

#[test]
fn returning_a_borrowed_parameter_slot_is_rejected() {
    check_errors("func f(p: *i32) -> **i32 {\n  &p\n}\n").expect("E5004", "local");
}

#[test]
fn returning_a_copy_of_a_parameter_reference_is_allowed() {
    check_clean("func f(p: *i32) -> *i32 {\n  let q = p;\n  q\n}\n");
}
