//! Borrowing places and reading through references.

use super::support::{check_clean, check_errors, expect_ty, expr_ty, function, tail};

#[test]
fn borrowing_a_local_produces_a_reference() {
    let (fixture, hir, results) = check_clean("func f() -> *i32 { let x = 1; &x }");
    let (_, body) = function(&fixture, &hir, "f");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "*i32");
}

#[test]
fn dereferencing_reads_the_pointee() {
    let (fixture, hir, results) = check_clean("func f(p: *i32) -> i32 { *p }");
    let (_, body) = function(&fixture, &hir, "f");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "i32");
}

#[test]
fn nested_references_lose_one_level_per_deref() {
    let (fixture, hir, results) = check_clean("func f(p: **i32) -> *i32 { *p }");
    let (_, body) = function(&fixture, &hir, "f");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "*i32");
}

#[test]
fn borrowing_string_data_keeps_the_reference() {
    let (fixture, hir, results) = check_clean("func f(s: *str) -> *str { &*s }");
    let (_, body) = function(&fixture, &hir, "f");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "*str");
}

#[test]
fn borrowing_through_a_reference_value_keeps_the_pointee_type() {
    let (fixture, hir, results) = check_clean(
        "func identity(p: *i32) -> *i32 { p }\nfunc f(p: *i32) -> *i32 { &*identity(p) }",
    );
    let (_, body) = function(&fixture, &hir, "f");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "*i32");
}

#[test]
fn borrowing_a_dereference_of_a_non_reference_is_rejected() {
    let (fixture, _, _) = check_errors("func f() -> i32 { let x = 1; &*x }");
    fixture.expect("E3012", "cannot dereference");
}

#[test]
fn exclusive_borrows_produce_exclusive_references() {
    let (fixture, hir, results) = check_clean("func f(p: *mut i32) -> *mut i32 { &mut *p }");
    let (_, body) = function(&fixture, &hir, "f");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "*mut i32");
}

#[test]
fn reference_kinds_are_not_interchangeable() {
    let (fixture, _, _) = check_errors("func f(p: *mut i32) -> *i32 { p }");
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn borrow_of_a_value_is_rejected() {
    let (fixture, _, _) = check_errors("func f() -> *i32 { &(1 + 2) }");
    fixture.expect("E3013", "cannot borrow");
}

#[test]
fn dereference_of_a_non_reference_is_rejected() {
    let (fixture, _, _) = check_errors("func f() -> i32 { let x = 1; *x }");
    fixture.expect("E3012", "cannot dereference");
}

#[test]
fn writing_through_a_shared_reference_is_rejected() {
    let (fixture, _, _) = check_errors("func f(p: *i32) { *p = 1; }");
    fixture.expect("E3014", "shared reference");
}

#[test]
fn storing_a_dereferenced_str_is_rejected() {
    let (fixture, _, _) = check_errors("func f(s: *str) { let x = *s; }");
    fixture.expect("E3010", "unsized");
}

#[test]
fn borrowing_an_uninitialized_binding_is_rejected() {
    let (fixture, _, _) = check_errors("func f() -> *i32 { let x: i32; &x }");
    fixture.expect("E3011", "possibly uninitialized");
}

#[test]
fn function_items_cannot_be_borrowed() {
    let (fixture, _, _) = check_errors("func g() {}\nfunc f() { let r = &g; }");
    fixture.expect("E3007", "function item");
}
