//! Operator tests.

use super::support::{
    check_clean, check_errors, expect_ty, expr_statement, expr_ty, function, statements, tail,
};

#[test]
fn arithmetic_propagates_the_expected_type() {
    let (fixture, hir, results) = check_clean("func main() -> i64 { 1 + 2 }");
    let (_, body) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "i64");
}

#[test]
fn mixed_width_arithmetic_is_rejected() {
    let (fixture, _, _) = check_errors("func main() -> i32 { let x = 1i32 + 1i64; 0 }");
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn arithmetic_on_bool_is_rejected() {
    let (fixture, _, _) = check_errors("func main() -> i32 { let x = true + false; 0 }");
    fixture.expect("E3002", "operator `+`");
}

#[test]
fn comparisons_produce_bool() {
    let (fixture, hir, results) = check_clean("func main() -> bool { 1 < 2 }");
    let (_, body) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "bool");
}

#[test]
fn ordering_bool_is_rejected() {
    let (fixture, _, _) = check_errors("func main() -> bool { true < false }");
    fixture.expect("E3002", "operator `<`");
}

#[test]
fn logical_operators_require_bool() {
    let (fixture, _, _) = check_errors("func main() -> bool { 1 && true }");
    fixture.expect("E3002", "logical operator `&&`");
}

#[test]
fn bitwise_operators_require_integers() {
    let (fixture, _, _) = check_errors("func main() -> bool { true & false }");
    fixture.expect("E3002", "operator `&`");
}

#[test]
fn shifts_require_matching_integer_types() {
    let (fixture, _, _) = check_errors("func main() -> i32 { 1 << true }");
    fixture.expect("E3001", "mismatched types");

    let (fixture, hir, results) = check_clean("func main() -> u8 { 1u8 << 4u8 }");
    let (_, body) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "u8");
}

#[test]
fn unary_negation_requires_a_signed_or_float_type() {
    let (fixture, hir, results) = check_clean("func main() -> f32 { -1.5 }");
    let (_, body) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "f32");

    let (fixture, _, _) = check_errors("func main() -> u8 { -1u8 }");
    fixture.expect("E3002", "cannot negate");
}

#[test]
fn logical_not_and_bitwise_not_have_their_own_families() {
    let (fixture, hir, results) = check_clean("func main() -> bool { !false }");
    let (_, body) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "bool");

    let (fixture, hir, results) = check_clean("func main() -> u8 { ~0u8 }");
    let (_, body) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "u8");

    let (fixture, _, _) = check_errors("func main() -> bool { !1 }");
    fixture.expect("E3002", "`!` requires a `bool`");
}

#[test]
fn compound_assignment_checks_the_operator_family() {
    let (fixture, _, _) = check_errors("func main() { let mut flag = true; flag &= false; }");
    fixture.expect("E3002", "compound assignment `&=`");

    let (fixture, hir, results) =
        check_clean("func main() -> i32 { let mut x = 1; x += 2; x <<= 1; x }");
    let (_, body) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "i32");
}

#[test]
fn assignments_and_updates_have_unit_type() {
    let (fixture, hir, results) = check_clean("func main() { let mut x = 0; x = 1; }");
    let (_, body) = function(&fixture, &hir, "main");
    let assignment = expr_statement(&hir, statements(&hir, body)[1]);
    expect_ty(&hir, expr_ty(&results, assignment), "()");
}

#[test]
fn increment_requires_a_mutable_numeric_place() {
    let (fixture, _, _) = check_errors("func main() { let flag = true; flag++; }");
    fixture.expect("E3004", "not declared as mutable");
}
