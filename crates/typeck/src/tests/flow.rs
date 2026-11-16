//! Conditional, loop, and return type tests.

use super::support::{
    check_clean, check_errors, expect_ty, expr_ty, function, local_ty, statements, tail,
};

#[test]
fn non_bool_conditions_are_rejected() {
    let (fixture, _, _) = check_errors("func main() { if 1 { } }");
    fixture.expect("E3003", "expected `bool`");

    let (fixture, _, _) = check_errors("func main() { while 1 { } }");
    fixture.expect("E3003", "expected `bool`");
}

#[test]
fn if_branches_must_unify() {
    let (fixture, _, _) = check_errors("func main() -> i32 { if true { 1 } else { false } }");
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn if_without_else_requires_a_unit_branch() {
    let (fixture, _, _) = check_errors("func main() { if true { 1 } }");
    fixture.expect("E3001", "requires the branch to have type `()`");
}

#[test]
fn an_if_expression_takes_the_expected_type() {
    let (fixture, hir, results) =
        check_clean("func main() -> i64 { let value: i64 = if true { 1 } else { 2 }; value }");
    let (_, body) = function(&fixture, &hir, "main");
    let binding = crate::tests::support::let_binding(&hir, statements(&hir, body)[0]);
    expect_ty(&hir, local_ty(&results, binding), "i64");
}

#[test]
fn returns_must_match_the_declared_type() {
    let (fixture, _, _) = check_errors("func main() -> i32 { return true; }");
    fixture.expect("E3001", "mismatched types");

    let (fixture, _, _) = check_errors("func main() -> i32 { return; }");
    fixture.expect("E3001", "mismatched types");

    let (fixture, _, _) = check_errors("func main() { return 1; }");
    fixture.expect("E3001", "mismatched types");

    let (fixture, _, _) = check_clean("func main() { return; }");
    let _ = fixture;
}

#[test]
fn a_diverging_branch_does_not_restrict_the_other_branch() {
    let (fixture, hir, results) = check_clean("func main() -> i32 { if true { return 1; } 2 }");
    let (_, body) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, body)), "i32");
}

#[test]
fn a_diverging_else_branch_is_compatible_with_a_value() {
    let (fixture, hir, results) =
        check_clean("func main() -> i32 { let value = if true { 1 } else { return 0; }; value }");
    let (_, body) = function(&fixture, &hir, "main");
    let binding = crate::tests::support::let_binding(&hir, statements(&hir, body)[0]);
    expect_ty(&hir, local_ty(&results, binding), "i32");
}

#[test]
fn both_branches_may_diverge() {
    let (fixture, _, _) =
        check_clean("func main() -> i32 { if true { return 1; } else { return 2; } }");
    let _ = fixture;
}

#[test]
fn a_while_body_is_checked_against_no_expected_type() {
    let (fixture, _, _) = check_clean("func main() { while true { 1 } }");
    let _ = fixture;
}
