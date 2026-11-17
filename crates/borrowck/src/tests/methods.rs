//! Borrow checking around modifying intrinsic methods.

use super::support::{check_clean, check_errors};

#[test]
fn an_automatic_exclusive_receiver_conflicts_with_a_live_shared_borrow() {
    let fixture = check_errors(
        "struct Point { x: i32 } \
         impl Point { func shift(self: *mut Point) { (*self).x += 1; } } \
         func f() -> i32 { \
             let mut point = Point { x: 0 }; \
             let view = &point; \
             point.shift(); \
             (*view).x \
         }",
    );
    fixture.expect("E5008", "already borrowed");
}

#[test]
fn an_automatic_shared_receiver_conflicts_with_a_live_exclusive_borrow() {
    let fixture = check_errors(
        "struct Point { x: i32 } \
         impl Point { func read(self: *Point) -> i32 { (*self).x } } \
         func f() -> i32 { \
             let mut point = Point { x: 0 }; \
             let view = &mut point; \
             let value = point.read(); \
             value + (*view).x \
         }",
    );
    fixture.expect("E5008", "already borrowed");
}

#[test]
fn a_method_receiver_may_borrow_after_the_previous_loan_ends() {
    check_clean(
        "struct Point { x: i32 } \
         impl Point { \
             func read(self: *Point) -> i32 { (*self).x } \
             func shift(self: *mut Point) { (*self).x += 1; } \
         } \
         func f() -> i32 { \
             let mut point = Point { x: 0 }; \
             let view = &point; \
             let value = view.read(); \
             point.shift(); \
             value + point.read() \
         }",
    );
}

#[test]
fn a_value_receiver_moves_the_struct() {
    check_clean(
        "struct Token { value: i32 } \
         impl Token { func consume(self: Token) -> i32 { self.value } } \
         func f() -> i32 { Token { value: 41 }.consume() + 1 }",
    );
}

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
