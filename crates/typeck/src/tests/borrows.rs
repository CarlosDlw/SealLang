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
    fixture.expect("E3013", "already a pointer");
}

#[test]
fn mutably_borrowing_an_immutable_binding_is_rejected() {
    let (fixture, _, _) = check_errors("func f() { let x = 1; let r = &mut x; }");
    fixture.expect("E3017", "not declared as mutable");
}

#[test]
fn mutably_borrowing_through_a_shared_reference_is_rejected() {
    let (fixture, _, _) = check_errors("func f(p: *i32) { let r = &mut *p; }");
    fixture.expect("E3017", "shared reference");
}

#[test]
fn mutably_borrowing_a_mutable_binding_produces_an_exclusive_reference() {
    let (fixture, hir, results) = check_clean("func f() -> *mut i32 { let mut x = 1; &mut x }");
    let (_, body) = function(&fixture, &hir, "f");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "*mut i32");
}

#[test]
fn writing_through_an_exclusive_reference_is_accepted() {
    check_clean("func f(p: *mut i32) { *p = 1; *p += 2; }");
}

#[test]
fn moving_out_of_a_reference_is_rejected() {
    let (fixture, _, _) = check_errors("struct P { x: i32 }\nfunc f(p: *P) -> P { *p }");
    fixture.expect("E3018", "cannot move out of a reference");
}

#[test]
fn borrowing_a_non_copy_pointee_is_accepted() {
    check_clean("struct P { x: i32 }\nfunc f(p: *P) -> *P { &*p }");
}

#[test]
fn moving_an_element_out_of_a_slice_is_rejected() {
    let (fixture, _, _) = check_errors("struct P { x: i32 }\nfunc f(p: *[P]) -> P { p[0] }");
    fixture.expect("E3018", "cannot move out of a reference");
}

#[test]
fn borrowing_an_element_of_a_slice_is_accepted() {
    check_clean("struct P { x: i32 }\nfunc f(p: *[P]) -> *P { &p[0] }");
}

#[test]
fn moving_an_element_out_of_an_array_behind_a_reference_is_rejected() {
    for reference in ["*[P; 2]", "*mut [P; 2]"] {
        let source = "struct P { x: i32 }\nfunc f(p: REF) -> P { p[0] }";
        let (fixture, _, _) = check_errors(&source.replace("REF", reference));
        fixture.expect("E3018", "cannot move out of a reference");
    }
}

#[test]
fn borrowing_an_element_of_an_array_behind_a_reference_is_accepted() {
    for reference in ["*[P; 2]", "*mut [P; 2]"] {
        let source = "struct P { x: i32 }\nfunc f(p: REF) -> *P { &p[0] }";
        check_clean(&source.replace("REF", reference));
    }
}

#[test]
fn mutably_borrowing_a_field_of_a_mutable_binding_is_accepted() {
    check_clean(
        "struct P { x: i32, y: i32 }\n\
         func f(p: *mut P) { let r = &mut (*p).x; *r = 1; }",
    );
}
