//! Call tests.

use super::support::{check_clean, check_errors, expect_ty, expr_ty, function, tail};

#[test]
fn calls_use_the_declared_signature() {
    let (fixture, hir, results) = check_clean(
        "func add(a: i32, b: i32) -> i32 { a + b } \
         func main() -> i32 { add(1, 2) }",
    );
    let (_, main) = function(&fixture, &hir, "main");
    expect_ty(&hir, expr_ty(&results, tail(&hir, main)), "i32");

    let (add, body) = function(&fixture, &hir, "add");
    let signature = results.signature(add).expect("add has a signature");
    assert_eq!(signature.params.len(), 2);
    expect_ty(&hir, signature.ret, "i32");
    assert_eq!(body.params.len(), 2);
}

#[test]
fn calls_work_before_the_declaration() {
    let (fixture, _, _) = check_clean("func main() -> i32 { helper() } func helper() -> i32 { 1 }");
    let _ = fixture;
}

#[test]
fn wrong_arity_is_reported_for_both_directions() {
    let (fixture, _, _) = check_errors("func f(value: i32) {} func main() { f(); f(1, 2); }");
    let count = fixture
        .codes()
        .iter()
        .filter(|code| code.as_str() == "E3008")
        .count();
    assert_eq!(count, 2, "both calls have the wrong arity");
}

#[test]
fn argument_types_must_match_the_parameters() {
    let (fixture, _, _) = check_errors("func f(value: i32) {} func main() { f(true); }");
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn a_local_binding_is_not_callable() {
    let (fixture, _, _) = check_errors("func main() { let value = 1; value(); }");
    fixture.expect("E3006", "not callable");
}

#[test]
fn a_function_item_can_be_stored_as_a_value() {
    let (fixture, _, _) = check_clean("func f() {} func main() { let g = f; }");
    let _ = fixture;
}

#[test]
fn a_unit_call_cannot_be_used_as_a_value() {
    let (fixture, _, _) = check_errors("func f() {} func main() -> i32 { f() }");
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn a_unit_call_is_fine_as_a_statement() {
    let (fixture, _, _) = check_clean("func f() {} func main() { f(); }");
    let _ = fixture;
}
