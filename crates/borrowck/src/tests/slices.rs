//! Slice loans, mutable slices, reborrows, and escaping references.

use super::support::{check_clean, check_errors};

#[test]
fn a_shared_slice_blocks_writes_until_its_last_use() {
    let fixture = check_errors(
        "func f() -> i32 { let mut values = [1, 2]; let slice = &values[..]; \
         values[0] = 3; slice[0] }",
    );
    fixture.expect("E5001", "borrowed");
}

#[test]
fn reading_through_a_shared_slice_is_allowed() {
    check_clean("func f() -> i32 { let values = [1, 2]; let slice = &values[..]; slice[0] }");
}

#[test]
fn a_mutable_slice_writes_and_ends_before_the_source_is_read() {
    check_clean(
        "func f() -> i32 { let mut values = [1, 2]; let slice = &mut values[..]; \
         slice[0] = 5; values[0] }",
    );
}

#[test]
fn reading_the_source_while_a_mutable_slice_is_live_is_rejected() {
    let fixture = check_errors(
        "func f() -> i32 { let mut values = [1, 2]; let slice = &mut values[..]; \
         let first = values[0]; slice[0] + first }",
    );
    fixture.expect("E5007", "exclusively borrowed");
}

#[test]
fn reborrowing_an_exclusive_slice_is_allowed() {
    check_clean("func f(values: *mut [i32]) -> *mut [i32] { &mut values[..] }");
}

#[test]
fn using_the_parent_while_a_slice_reborrow_is_live_is_rejected() {
    let fixture = check_errors(
        "func f(values: *mut [i32]) -> i32 { let slice = &mut values[..]; \
         let first = values[0]; slice[0] + first }",
    );
    fixture.expect("E5007", "exclusively borrowed");
}

#[test]
fn returning_a_slice_of_a_local_array_is_rejected() {
    check_errors("func f() -> *[i32] { let values = [1, 2]; &values[..] }")
        .expect("E5004", "local");
}

#[test]
fn len_ends_an_exclusive_slice_loan() {
    check_clean(
        "func f() -> i32 { let mut values = [1, 2]; let slice = &mut values[..]; \
         if len(slice) == 2 { values[0] } else { 0 } }",
    );
}
