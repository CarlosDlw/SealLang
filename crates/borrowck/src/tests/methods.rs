//! Borrow checking around modifying intrinsic methods.

use super::support::{check_clean, check_errors};

#[test]
fn a_modifying_method_conflicts_with_a_live_shared_borrow() {
    let fixture =
        check_errors("func f() -> i32 { let mut a = [1, 2, 3]; let r = &a; a.reverse(); r[0] }");
    fixture.expect("E5008", "already borrowed");
}

#[test]
fn a_modifying_method_is_allowed_after_the_shared_borrow_ends() {
    check_clean(
        "func f() -> i32 { let mut a = [1, 2, 3]; let r = &a; let x = r[0]; a.reverse(); x }",
    );
}

#[test]
fn an_exclusive_slice_receiver_permits_modification() {
    check_clean("func f(values: *mut [i32]) { values.reverse(); }");
}

#[test]
fn an_exclusive_scalar_receiver_permits_toggle() {
    check_clean("func f(flag: *mut bool) { flag.toggle(); }");
}

#[test]
fn an_exclusive_array_reference_permits_modification() {
    check_clean("func f(values: *mut [i32; 3]) { values.fill(1); }");
}
