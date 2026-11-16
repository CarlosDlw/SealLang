//! Shared-loan conflicts and borrow precision.

use super::support::{check_clean, check_errors};

#[test]
fn writing_after_the_last_use_is_allowed() {
    check_clean(
        "func f() -> i32 {\n  let mut x = 1;\n  let r = &x;\n  let y = *r;\n  x = 2;\n  y\n}\n",
    );
}

#[test]
fn writing_while_a_borrow_is_live_is_rejected() {
    let fixture =
        check_errors("func f() -> i32 {\n  let mut x = 1;\n  let r = &x;\n  x = 2;\n  *r\n}\n");
    fixture.expect("E5001", "borrowed");
}

#[test]
fn two_shared_borrows_may_overlap() {
    check_clean("func f() -> i32 {\n  let x = 1;\n  let a = &x;\n  let b = &x;\n  *a + *b\n}\n");
}

#[test]
fn copying_a_reference_extends_the_loan() {
    let fixture = check_errors(
        "func f() -> i32 {\n  let mut x = 1;\n  let r = &x;\n  let s = r;\n  x = 2;\n  *s\n}\n",
    );
    fixture.expect("E5001", "borrowed");
}

#[test]
fn a_write_on_one_branch_conflicts_with_a_live_borrow() {
    let fixture = check_errors(
        "func f(c: bool) -> i32 {\n  let mut x = 1;\n  let r = &x;\n  if c {\n    x = 2;\n  }\n  *r\n}\n",
    );
    fixture.expect("E5001", "borrowed");
}

#[test]
fn a_write_inside_a_loop_conflicts_with_a_live_borrow() {
    let fixture = check_errors(
        "func f() -> i32 {\n  let mut x = 1;\n  let r = &x;\n  while x < 2 {\n    x = 2;\n  }\n  *r\n}\n",
    );
    fixture.expect("E5001", "borrowed");
}

#[test]
fn overwriting_the_reference_ends_the_old_loan() {
    check_clean(
        "func f() -> i32 {\n  let mut x = 1;\n  let y = 2;\n  let mut r = &x;\n  r = &y;\n  x = 3;\n  *r\n}\n",
    );
}

#[test]
fn a_branch_that_reborrows_keeps_both_loans_alive() {
    let fixture = check_errors(
        "func f(c: bool) -> i32 {\n  let mut x = 1;\n  let y = 2;\n  let mut r = &y;\n  if c {\n    r = &x;\n  }\n  x = 3;\n  *r\n}\n",
    );
    fixture.expect("E5001", "borrowed");
}
