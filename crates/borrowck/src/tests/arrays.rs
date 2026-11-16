//! Array moves, borrows, and initialization through move paths.

use super::support::{check_clean, check_errors};

#[test]
fn borrowing_one_element_keeps_the_array_usable() {
    check_clean("func f() -> i32 { let a = [1, 2, 3]; let r = &a[0]; *r }");
}

#[test]
fn writing_through_an_exclusive_element_borrow_is_allowed() {
    check_clean("func f() -> i32 { let mut a = [1, 2]; let r = &mut a[1]; *r = 3; a[1] }");
}

#[test]
fn element_storage_aliases_conservatively() {
    let fixture =
        check_errors("func f() -> i32 { let mut a = [1, 2]; let r = &a[0]; a[1] = 3; *r }");
    fixture.expect("E5001", "borrowed");
}

#[test]
fn moving_an_element_then_reading_a_sibling_is_allowed() {
    check_clean(
        "struct Point { x: i32 } \
         func f() -> i32 { let a = [Point { x: 1 }, Point { x: 2 }]; \
         let first = a[0]; let second = a[1]; first.x + second.x }",
    );
}

#[test]
fn moving_the_whole_array_after_an_element_move_is_rejected() {
    let fixture = check_errors(
        "struct Point { x: i32 } \
         func take(a: [Point; 2]) -> i32 { a[0].x } \
         func f() -> i32 { let a = [Point { x: 1 }, Point { x: 2 }]; \
         let first = a[0]; take(a) + first.x }",
    );
    fixture.expect("E5005", "`a`");
}

#[test]
fn an_array_of_references_carries_its_loans() {
    check_clean("func f() -> i32 { let x = 1; let y = 2; let a = [&x, &y]; let r = a[1]; *r }");
}
