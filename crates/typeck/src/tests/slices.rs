//! Slice types, array-to-slice coercion, ranges, indexing, and `len`.

use crate::PlaceProjection;

use super::support::{
    check_clean, check_errors, expect_ty, expr_ty, function, let_binding, local_ty, statements,
    tail,
};

#[test]
fn an_array_reference_coerces_when_a_slice_is_expected() {
    let (fixture, hir, results) =
        check_clean("func f() { let values = [1, 2]; let slice: *[i32] = &values; }");
    let (_, body) = function(&fixture, &hir, "f");
    let binding = let_binding(&hir, statements(&hir, body)[1]);
    expect_ty(&hir, local_ty(&results, binding), "*[i32]");
}

#[test]
fn an_unconstrained_array_reference_keeps_the_array_type() {
    let (fixture, hir, results) =
        check_clean("func f() { let values = [1, 2]; let reference = &values; }");
    let (_, body) = function(&fixture, &hir, "f");
    let binding = let_binding(&hir, statements(&hir, body)[1]);
    expect_ty(&hir, local_ty(&results, binding), "*[i32; 2]");
}

#[test]
fn ranges_produce_shared_and_exclusive_slice_references() {
    let (fixture, hir, results) = check_clean(
        "func shared() -> *[i32] { let values = [1, 2, 3]; &values[1..] }\n\
         func exclusive() -> *mut [i32] { let mut values = [1, 2, 3]; &mut values[..2] }",
    );
    let (_, body) = function(&fixture, &hir, "shared");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "*[i32]");

    let (_, body) = function(&fixture, &hir, "exclusive");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "*mut [i32]");
}

#[test]
fn a_slice_can_be_sub_sliced_and_measured() {
    let (fixture, hir, results) = check_clean(
        "func subslice(values: *[i32]) -> *[i32] { &values[1..] }\n\
         func length(values: *[i32]) -> usize { len(values) }",
    );
    let (_, body) = function(&fixture, &hir, "subslice");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "*[i32]");

    let (_, body) = function(&fixture, &hir, "length");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "usize");
}

#[test]
fn slice_indexing_auto_dereferences_and_keeps_a_runtime_length() {
    let (fixture, hir, results) =
        check_clean("func f(values: *[i32], index: usize) -> i32 { values[index] }");
    let (_, body) = function(&fixture, &hir, "f");
    let expression = tail(&hir, body);
    expect_ty(&hir, expr_ty(&results, expression), "i32");
    let place = results.place(expression).expect("a slice index is a place");
    assert!(matches!(
        place.projections.as_slice(),
        [
            PlaceProjection::Deref,
            PlaceProjection::Index {
                constant: None,
                dynamic_length: true,
                ..
            }
        ]
    ));
}

#[test]
fn writing_through_an_exclusive_slice_reference_is_accepted() {
    check_clean("func f(values: *mut [i32]) { values[0] = 1; }");
}

#[test]
fn a_bare_range_is_rejected() {
    let (fixture, _, _) = check_errors("func f() { let values = [1, 2]; values[..]; }");
    fixture.expect("E3024", "must be borrowed");
}

#[test]
fn a_slice_binding_is_unsized() {
    let (fixture, _, _) = check_errors("func f(values: [i32]) {}");
    fixture.expect("E3010", "unsized");

    let (fixture, _, _) = check_errors("func f() { let values: [i32]; }");
    fixture.expect("E3010", "unsized");
}

#[test]
fn len_requires_a_slice_reference_and_one_argument() {
    let (fixture, _, _) = check_errors("func f() -> usize { len(1) }");
    fixture.expect("E3025", "slice reference");

    let (fixture, _, _) = check_errors("func f() -> usize { len() }");
    fixture.expect("E2014", "exactly one argument");
}

#[test]
fn constant_range_bounds_are_checked() {
    let (fixture, _, _) = check_errors("func f() -> *[i32] { let values = [1, 2]; &values[1..0] }");
    fixture.expect("E3026", "start is greater");

    let (fixture, _, _) = check_errors("func f() -> *[i32] { let values = [1, 2]; &values[..3] }");
    fixture.expect("E3022", "out of bounds");
}

#[test]
fn a_mutable_slice_requires_a_writable_source() {
    let (fixture, _, _) =
        check_errors("func f() -> *mut [i32] { let values = [1, 2]; &mut values[..] }");
    fixture.expect("E3017", "not declared as mutable");

    let (fixture, _, _) = check_errors("func f(values: *[i32]) -> *mut [i32] { &mut values[..] }");
    fixture.expect("E3017", "shared reference");
}

#[test]
fn a_slice_reference_cannot_be_dereferenced_as_a_value() {
    let (fixture, _, _) = check_errors("func f(values: *[i32]) { let copy = *values; }");
    fixture.expect("E3024", "cannot read");
}

#[test]
fn slices_of_structs_keep_their_element_type() {
    let (fixture, hir, results) = check_clean(
        "struct Point { x: i32 }\n\
         func f(points: *[Point]) -> i32 { points[0].x }",
    );
    let (_, body) = function(&fixture, &hir, "f");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "i32");
}
