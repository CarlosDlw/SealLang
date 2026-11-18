//! Numeric casts read their operand without creating a loan.

use super::support::check_clean;

#[test]
fn a_cast_does_not_extend_a_reference_loan() {
    check_clean(
        "func f() -> i32 {\n  let mut x = 1;\n  let r = &x;\n  let y = (*r) as i64;\n  x = 2;\n  y as i32\n}\n",
    );
}

#[test]
fn a_cast_does_not_keep_the_operand_borrowed() {
    check_clean(
        "func f() -> i64 {\n  let mut x = 1;\n  let r = &mut x;\n  *r = 2;\n  x as i64\n}\n",
    );
}

#[test]
fn a_cast_reads_an_initialized_numeric_local() {
    check_clean("func f(value: u32) -> u64 {\n  let converted = value as u64;\n  converted\n}\n");
}

#[test]
fn casting_the_same_copyable_operand_twice_is_allowed() {
    check_clean(
        "func f(value: i32) -> i64 {\n  let first = value as i64;\n  let second = value as i64;\n  first + second\n}\n",
    );
}
