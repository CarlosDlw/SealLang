//! C scalar aliases take part in ordinary type unification.

use super::support::{
    check_clean, check_errors, expect_ty, function, let_binding, local_ty, statements,
};

#[test]
fn c_int_is_i32() {
    let (fixture, hir, results) =
        check_clean("func main() -> i32 { let value: c_int = 1; let same: i32 = value; same }");
    let (_, body) = function(&fixture, &hir, "main");
    let binding = let_binding(&hir, statements(&hir, body)[0]);
    expect_ty(&hir, local_ty(&results, binding), "i32");
}

#[test]
fn c_long_is_64_bits_on_the_default_abi() {
    let (fixture, _, _) = check_errors(
        "func main() -> i32 { let value: c_long = 1; let narrow: i32 = value; narrow }",
    );
    assert!(
        fixture.codes().iter().any(|code| code == "E3001"),
        "assigning `c_long` to `i32` must be a type mismatch: {:?}",
        fixture.codes()
    );
}

#[test]
fn a_signed_c_char_accepts_a_negative_literal() {
    check_clean("func main() -> i32 { let value: c_char = -1; value as i32 }");
}
