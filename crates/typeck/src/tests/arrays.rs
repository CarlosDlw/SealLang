//! Fixed-size arrays: literals, repeats, indexing, and initialization.

use super::support::{
    Fixture, check_clean, check_errors, check_with_width, expect_ty, expr_ty, function,
    let_binding, local_ty, statements, tail, ty_text_with,
};
use crate::PlaceProjection;

#[test]
fn array_literals_have_the_annotation_and_constant_indices_are_places() {
    let (fixture, hir, results) =
        check_clean("func main() -> i32 { let a: [i32; 3] = [1, 2, 3]; a[1] }");
    let (_, body) = function(&fixture, &hir, "main");
    let binding = let_binding(&hir, statements(&hir, body)[0]);
    expect_ty(&hir, local_ty(&results, binding), "[i32; 3]");
    let index = tail(&hir, body);
    expect_ty(&hir, expr_ty(&results, index), "i32");
    let place = results.place(index).expect("index is a place");
    assert!(matches!(
        place.projections.as_slice(),
        [PlaceProjection::Index {
            constant: Some(1),
            length: 3,
            ..
        }]
    ));
}

#[test]
fn empty_arrays_require_an_array_annotation() {
    let (fixture, _, _) = check_errors("func main() { let a = []; }");
    fixture.expect("E3019", "cannot infer the element type");

    check_clean("func main() { let a: [i32; 0] = []; }");
}

#[test]
fn an_empty_array_with_a_non_array_annotation_is_a_mismatch() {
    let (fixture, _, _) = check_errors("func main() { let a: i32 = []; }");
    fixture.expect("E3001", "mismatched types");
    assert!(
        !fixture.codes().iter().any(|code| code == "E3019"),
        "a known non-array annotation must not ask for an element type"
    );
}

#[test]
fn array_lengths_must_match_the_annotation() {
    let (fixture, _, _) = check_errors("func main() { let a: [i32; 2] = [1, 2, 3]; }");
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn repeated_arrays_require_a_copy_element() {
    let (fixture, _, _) = check_errors(
        "struct Point { x: i32 } \
         func main() { let p = Point { x: 1 }; let a = [p; 2]; }",
    );
    fixture.expect("E3020", "not `Copy`");

    let (fixture, _, _) = check_errors(
        "struct Point { x: i32 } \
         func main() { let p = Point { x: 1 }; let a = [p; 0]; }",
    );
    fixture.expect("E3020", "not `Copy`");
}

#[test]
fn repeated_scalars_and_copy_arrays_are_accepted() {
    let (fixture, hir, results) =
        check_clean("func main() -> i32 { let a = [7; 4]; let b = [a; 2]; b[1][3] }");
    let (_, body) = function(&fixture, &hir, "main");
    let binding = let_binding(&hir, statements(&hir, body)[0]);
    expect_ty(&hir, local_ty(&results, binding), "[i32; 4]");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "i32");
}

#[test]
fn constant_indices_are_bounds_checked_at_compile_time() {
    let (fixture, _, _) = check_errors("func main() -> i32 { let a = [1, 2]; a[2] }");
    fixture.expect("E3022", "out of bounds");

    let (fixture, _, _) = check_errors("func main() -> i32 { let a: [i32; 0] = []; a[0] }");
    fixture.expect("E3022", "out of bounds");
}

#[test]
fn dynamic_indices_must_be_usize() {
    let (fixture, _, _) =
        check_errors("func main() -> i32 { let a = [1, 2]; let i: i32 = 0; a[i] }");
    fixture.expect("E3001", "mismatched types");

    let (fixture, hir, results) =
        check_clean("func main() -> i32 { let a = [1, 2]; let i: usize = 1; a[i] }");
    let (_, body) = function(&fixture, &hir, "main");
    let index = tail(&hir, body);
    let place = results.place(index).expect("index is a place");
    assert!(matches!(
        place.projections.as_slice(),
        [PlaceProjection::Index { constant: None, .. }]
    ));
}

#[test]
fn only_arrays_can_be_indexed() {
    let (fixture, _, _) = check_errors("func main() -> i32 { let value = 1; value[0] }");
    fixture.expect("E3021", "cannot index a value");
}

#[test]
fn indexing_requires_a_place() {
    let (fixture, _, _) = check_errors("func main() -> i32 { [1, 2][0] }");
    fixture.expect("E2013", "indexing requires a place");
}

#[test]
fn array_elements_initialize_independently() {
    check_clean("func main() -> i32 { let mut a: [i32; 2]; a[0] = 1; a[1] = 2; a[0] + a[1] }");

    let (fixture, _, _) =
        check_errors("func main() -> i32 { let mut a: [i32; 2]; a[0] = 1; a[1] }");
    fixture.expect("E3011", "`a[1]` is possibly uninitialized");

    let (fixture, _, _) = check_errors("func main() -> i32 { let mut a: [i32; 2]; a[0] = 1; a }");
    fixture.expect("E3011", "`a` is possibly uninitialized");
}

#[test]
fn a_whole_array_assignment_initializes_every_element() {
    check_clean("func main() -> i32 { let mut a: [i32; 3]; a = [1, 2, 3]; a[2] }");
}

#[test]
fn a_dynamic_write_requires_an_initialized_array() {
    let (fixture, _, _) =
        check_errors("func main() { let mut a: [i32; 2]; let i: usize = 0; a[i] = 1; }");
    fixture.expect("E3011", "possibly uninitialized");
}

#[test]
fn a_dynamic_write_to_an_initialized_array_is_accepted() {
    check_clean("func main() { let mut a = [1, 2]; let i: usize = 0; a[i] = 3; }");
}

#[test]
fn arrays_of_structs_keep_the_element_type() {
    let (fixture, hir, results) = check_clean(
        "struct Point { x: i32 } \
         func main() -> i32 { let a = [Point { x: 1 }, Point { x: 2 }]; a[1].x }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let binding = let_binding(&hir, statements(&hir, body)[0]);
    assert_eq!(
        ty_text_with(&fixture, &hir, local_ty(&results, binding)),
        "[Point; 2]"
    );
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "i32");
}

#[test]
fn array_annotations_are_checked_against_the_target_address_space() {
    let (fixture, _, _) = check_with_width(
        "func main() { let a: [u16; 4294967295] = [0; 4294967295]; }",
        32,
    );
    fixture.expect("E3023", "too large");

    let fixture = Fixture::new("func main() { let a: [u16; 2] = [0; 2]; }");
    let _ = fixture.check(32);
    fixture.assert_clean();
}

#[test]
fn an_unstored_array_literal_is_also_checked_against_the_target() {
    let fixture = Fixture::new("func main() { [0; 4294967295]; }");
    let _ = fixture.check(32);
    fixture.expect("E3023", "too large");

    let fixture = Fixture::new("func main() { [[0; 4294967295]; 4294967295]; }");
    let _ = fixture.check(64);
    fixture.expect("E3023", "too large");
}
