//! Loan conflicts and borrow precision.

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
fn reading_through_a_borrow_expression_is_allowed() {
    check_clean("func f() -> i32 {\n  let x = 1;\n  *&x\n}\n");
}

#[test]
fn reading_through_a_returned_reference_is_allowed() {
    check_clean(
        "func identity(p: *i32) -> *i32 {\n  p\n}\n\n\
         func f() -> i32 {\n  let x = 2;\n  *identity(&x) + 1\n}\n",
    );
}

#[test]
fn borrowing_through_a_reference_value_keeps_the_pointee_alive() {
    check_clean(
        "func identity(p: *i32) -> *i32 {\n  p\n}\n\n\
         func f() -> i32 {\n  let x = 2;\n  let r = &*identity(&x);\n  *r + 1\n}\n",
    );
}

#[test]
fn a_read_of_a_field_names_the_moved_place() {
    let fixture = check_errors(
        "struct Point { x: i32, y: i32 }\n\
         func f() -> i32 {\n  let a = Point { x: 1, y: 2 };\n  let b = a;\n  a.x + b.y\n}\n",
    );
    fixture.expect("E5005", "because `a` was moved");
}

#[test]
fn a_read_of_a_whole_place_names_it_once() {
    let fixture = check_errors(
        "struct Point { x: i32, y: i32 }\n\
         func f() -> Point {\n  let a = Point { x: 1, y: 2 };\n  let b = a;\n  let c = a;\n  c\n}\n",
    );
    fixture.expect("E5005", "`a` was moved and cannot be used here");
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
fn reading_while_an_exclusive_borrow_is_live_is_rejected() {
    let fixture = check_errors(
        "func f() -> i32 {\n  let mut x = 1;\n  let r = &mut x;\n  let y = x;\n  *r + y\n}\n",
    );
    fixture.expect("E5007", "exclusively borrowed");
}

#[test]
fn writing_while_an_exclusive_borrow_is_live_is_rejected() {
    let fixture =
        check_errors("func f() -> i32 {\n  let mut x = 1;\n  let r = &mut x;\n  x = 2;\n  *r\n}\n");
    fixture.expect("E5008", "exclusively borrowed");
}

#[test]
fn moving_while_an_exclusive_borrow_is_live_is_rejected() {
    let fixture = check_errors(
        "struct Point { x: i32 }\n\
         func f() -> i32 {\n  let mut p = Point { x: 1 };\n  let r = &mut p;\n  let q = p;\n  (*r).x + q.x\n}\n",
    );
    fixture.expect("E5008", "exclusively borrowed");
}

#[test]
fn two_exclusive_borrows_of_the_same_place_conflict() {
    let fixture = check_errors(
        "func f() -> i32 {\n  let mut x = 1;\n  let a = &mut x;\n  let b = &mut x;\n  *a + *b\n}\n",
    );
    fixture.expect("E5008", "already borrowed");
}

#[test]
fn an_exclusive_borrow_conflicts_with_a_shared_one() {
    let fixture = check_errors(
        "func f() -> i32 {\n  let mut x = 1;\n  let a = &mut x;\n  let b = &x;\n  *a + *b\n}\n",
    );
    fixture.expect("E5008", "borrowed as mutable");
}

#[test]
fn writes_through_an_exclusive_borrow_are_allowed() {
    check_clean("func f() -> i32 {\n  let mut x = 1;\n  let r = &mut x;\n  *r = 2;\n  *r\n}\n");
}

#[test]
fn writing_after_the_exclusive_borrow_ends_is_allowed() {
    check_clean(
        "func f() -> i32 {\n  let mut x = 1;\n  let r = &mut x;\n  *r = 2;\n  x = 3;\n  x\n}\n",
    );
}

#[test]
fn overwriting_the_reference_ends_the_old_loan() {
    check_clean(
        "func f() -> i32 {\n  let mut x = 1;\n  let y = 2;\n  let mut r = &x;\n  r = &y;\n  x = 3;\n  *r\n}\n",
    );
}

#[test]
fn a_reborrow_through_an_exclusive_reference_is_allowed() {
    check_clean(
        "func f() -> i32 {\n  let mut x = 1;\n  let a = &mut x;\n  let b = &mut *a;\n  *b = 2;\n  *a\n}\n",
    );
}

#[test]
fn an_exclusive_borrow_of_the_pointee_conflicts() {
    let fixture = check_errors(
        "func f(p: *mut i32) -> i32 {\n  let a = &mut *p;\n  let b = &mut *p;\n  *a + *b\n}\n",
    );
    fixture.expect("E5008", "already borrowed");
}

#[test]
fn writing_the_pointee_conflicts_with_a_live_borrow() {
    let fixture =
        check_errors("func f(p: *mut i32) -> i32 {\n  let a = &mut *p;\n  *p = 1;\n  *a\n}\n");
    fixture.expect("E5008", "exclusively borrowed");
}

#[test]
fn reading_the_pointee_conflicts_with_an_exclusive_borrow() {
    let fixture = check_errors(
        "func f(p: *mut i32) -> i32 {\n  let a = &mut *p;\n  let v = *p;\n  *a + v\n}\n",
    );
    fixture.expect("E5007", "exclusively borrowed");
}

#[test]
fn a_copy_of_a_pointer_orders_its_borrows() {
    let fixture = check_errors(
        "func f(p: *mut i32) -> i32 {\n  let q = p;\n  let a = &mut *p;\n  let b = &mut *q;\n  *a + *b\n}\n",
    );
    fixture.expect("E5008", "already borrowed");
}

#[test]
fn a_shared_pointee_borrow_conflicts_with_an_exclusive_one() {
    let fixture = check_errors(
        "func f(p: *mut i32) -> i32 {\n  let a = &*p;\n  let b = &mut *p;\n  *a + *b\n}\n",
    );
    fixture.expect("E5008", "already borrowed");
}

#[test]
fn using_the_parent_while_a_reborrow_is_live_is_rejected() {
    let fixture = check_errors(
        "func f() -> i32 {\n  let mut x = 1;\n  let a = &mut x;\n  let b = &mut *a;\n  let v = *a;\n  *b + v\n}\n",
    );
    fixture.expect("E5007", "exclusively borrowed");
}

#[test]
fn field_borrows_of_a_pointee_stay_precise() {
    check_clean(
        "struct P { x: i32, y: i32 }\n\
         func f(p: *mut P) -> i32 {\n  let a = &mut (*p).x;\n  (*p).y = 1;\n  *a + (*p).y\n}\n",
    );
}

#[test]
fn a_rejected_borrow_does_not_cascade_into_reads() {
    let fixture = check_errors(
        "func f() -> i32 {\n  let mut x = 1;\n  let a = &mut x;\n  let b = &x;\n  *a + *b\n}\n",
    );
    let diagnostics = fixture.diagnostics();
    assert_eq!(
        diagnostics.len(),
        1,
        "only the rejected borrow is reported: {:?}",
        diagnostics
            .iter()
            .map(|diagnostic| (diagnostic.code(), diagnostic.message()))
            .collect::<Vec<_>>()
    );
    fixture.expect("E5008", "already borrowed as mutable");
}

#[test]
fn a_rejected_borrow_does_not_cascade_into_arguments() {
    let fixture = check_errors(
        "func take(p: *mut i32) {}\n\n\
         func f() -> i32 {\n  let mut x = 1;\n  let a = &mut x;\n  let b = &mut x;\n  take(b);\n  *a\n}\n",
    );
    let diagnostics = fixture.diagnostics();
    assert_eq!(
        diagnostics.len(),
        1,
        "only the rejected borrow is reported: {:?}",
        diagnostics
            .iter()
            .map(|diagnostic| (diagnostic.code(), diagnostic.message()))
            .collect::<Vec<_>>()
    );
    fixture.expect("E5008", "already borrowed");
}

#[test]
fn a_shared_pointee_read_is_allowed() {
    check_clean("func f(p: *mut i32) -> i32 {\n  let a = &*p;\n  let v = *p;\n  *a + v\n}\n");
}

#[test]
fn a_stored_shared_trait_receiver_conflicts_with_a_later_exclusive_borrow() {
    // `p.view()` receives `p` through a shared reborrow of the pointee. The
    // returned reference keeps that loan alive, so a later exclusive borrow of
    // the same storage must be rejected like any other overlapping loan.
    let fixture = check_errors(
        "struct Counter { value: i32 }\n\
         trait View { func view(self: *Self) -> *Self; }\n\
         impl View for Counter { func view(self: *Counter) -> *Counter { self } }\n\
         func f(p: *mut Counter) -> i32 {\n\
           let shared = p.view();\n\
           let exclusive = &mut *p;\n\
           (*exclusive).value = 1;\n\
           (*shared).value\n\
         }\n",
    );
    fixture.expect("E5008", "already borrowed");
}

#[test]
fn a_conflict_labels_the_last_use_of_the_live_borrow() {
    let fixture =
        check_errors("func f() -> i32 {\n  let mut x = 1;\n  let r = &mut x;\n  x = 2;\n  *r\n}\n");
    let diagnostic = fixture
        .diagnostics()
        .into_iter()
        .find(|diagnostic| diagnostic.code().map(|code| code.as_str()) == Some("E5008"))
        .expect("the assignment conflicts with the borrow");
    assert!(
        diagnostic
            .labels()
            .iter()
            .any(|label| label.message == "borrow is used here"),
        "expected a last-use label, got {:?}",
        diagnostic.labels()
    );
}

#[test]
fn a_branch_that_reborrows_keeps_both_loans_alive() {
    let fixture = check_errors(
        "func f(c: bool) -> i32 {\n  let mut x = 1;\n  let y = 2;\n  let mut r = &y;\n  if c {\n    r = &x;\n  }\n  x = 3;\n  *r\n}\n",
    );
    fixture.expect("E5001", "borrowed");
}
