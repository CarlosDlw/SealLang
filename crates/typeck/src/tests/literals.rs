//! Literal typing tests.

use super::support::{
    Fixture, check_clean, check_with_width, expect_ty, expr_ty, function, let_binding, local_ty,
    statements, tail,
};

#[test]
fn integer_literals_default_to_i32() {
    let (fixture, hir, results) = check_clean("func main() -> i32 { 1 }");
    let (_, body) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "i32");
}

#[test]
fn suffix_selects_the_literal_type() {
    let (fixture, hir, results) = check_clean("func main() -> u8 { 1u8 }");
    let (_, body) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "u8");
}

#[test]
fn float_literals_default_to_f64() {
    let (fixture, hir, results) = check_clean("func main() -> f64 { 1.5 }");
    let (_, body) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "f64");
}

#[test]
fn annotations_hint_integer_literals() {
    let (fixture, hir, results) = check_clean("func main() -> i64 { let x: i64 = 1; x }");
    let (_, body) = function(&fixture, &hir, "main");
    let binding = let_binding(&hir, statements(&hir, body)[0]);
    expect_ty(&hir, local_ty(&results, binding), "i64");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "i64");
}

#[test]
fn float_literals_adopt_f32_annotations() {
    let (fixture, hir, results) = check_clean("func main() -> f32 { let x: f32 = 3.43; x }");
    let (_, body) = function(&fixture, &hir, "main");
    let binding = let_binding(&hir, statements(&hir, body)[0]);
    expect_ty(&hir, local_ty(&results, binding), "f32");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "f32");
}

#[test]
fn string_literals_have_reference_to_str_type() {
    let (fixture, hir, results) = check_clean("func main() -> *str { let text = \"hi\"; text }");
    let (_, body) = function(&fixture, &hir, "main");
    let binding = let_binding(&hir, statements(&hir, body)[0]);
    expect_ty(&hir, local_ty(&results, binding), "*str");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "*str");
}

#[test]
fn an_integer_literal_outside_i32_is_reported() {
    let (fixture, _, _) = super::support::check_errors("func main() -> i32 { 3000000000 }");
    fixture.expect("E3009", "out of range for `i32`");
}

#[test]
fn an_unsigned_literal_outside_its_type_is_reported() {
    let (fixture, _, _) = super::support::check_errors("func main() -> u8 { 300 }");
    fixture.expect("E3009", "out of range for `u8`");
}

#[test]
fn isize_range_follows_the_target_pointer_width() {
    let source = "func main() -> isize { 3000000000 }";

    let (wide, wide_hir, wide_results) = check_with_width(source, 64);
    wide.assert_clean();
    let (_, body) = function(&wide, &wide_hir, "main");
    expect_ty(
        &wide_hir,
        expr_ty(&wide_results, tail(&wide_hir, body)),
        "isize",
    );

    let (narrow, _, _) = check_with_width(source, 32);
    narrow.expect("E3009", "out of range for `isize`");
}

#[test]
fn the_most_negative_signed_literal_is_accepted() {
    let (fixture, hir, results) = check_clean("func main() -> i8 { -128i8 }");
    let (_, body) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "i8");
}

#[test]
fn a_signed_literal_below_the_minimum_is_rejected() {
    let (fixture, _, _) = super::support::check_errors("func main() -> i8 { -129i8 }");
    fixture.expect("E3009", "out of range for `i8`");
}

#[test]
fn an_f32_literal_that_overflows_is_reported() {
    let (fixture, _, _) = super::support::check_errors("func main() -> f32 { 1e300 }");
    fixture.expect("E3009", "out of range for `f32`");
}

#[test]
fn a_suffixed_literal_does_not_adopt_the_context() {
    let (fixture, _, _) = super::support::check_errors("func main() -> i64 { 1i32 }");
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn the_fixture_keeps_lower_diagnostics_visible() {
    // A guard against silently treating a lowering error as a type error.
    let fixture = Fixture::new("func main() { missing }");
    let _ = fixture.check(64);
    fixture.expect("E2002", "cannot find `missing`");
}
