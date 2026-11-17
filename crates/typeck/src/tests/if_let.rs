//! Type checking `if let` expressions.

use super::support::{Fixture, check_clean};

#[test]
fn if_let_binds_refutable_pattern_arms_in_the_then_branch() {
    let (_, _, _) = check_clean(
        "enum Option { None, Some(i32) } \
         func main(value: Option) -> i32 { \
           if let Option::Some(inner) = value { inner } else { 0 } \
         }",
    );

    let (_, _, _) = check_clean(
        "func main(value: i32) -> i32 { \
           if let 0 = value { 1 } else if let 1 = value { 2 } else { 3 } \
         }",
    );
}

#[test]
fn if_let_without_else_requires_a_unit_branch() {
    let fixture = Fixture::new(
        "enum Option { None, Some(i32) } \
         func main(value: Option) { \
           if let Option::Some(inner) = value { inner } \
         }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3001", "`if let` without `else`");
}

#[test]
fn if_let_branches_must_agree_and_match_the_pattern_type() {
    let fixture = Fixture::new(
        "enum Option { None, Some(i32) } \
         func main(value: Option) -> i32 { \
           if let Option::Some(inner) = value { inner } else { false } \
         }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3001", "mismatched types");

    let fixture = Fixture::new(
        "enum Option { None, Some(i32) } \
         func main(value: i32) -> i32 { \
           if let Option::Some(inner) = value { inner } else { 0 } \
         }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn if_let_accepts_slice_patterns() {
    let (_, _, _) = check_clean(
        "func main(values: *[i32]) -> i32 { \
           if let [first, .., last] = values { first + last } else { 0 } \
         }",
    );
}

#[test]
fn if_let_accepts_irrefutable_patterns() {
    let (_, _, _) = check_clean(
        "func main(value: i32) -> i32 { \
           if let inner = value { inner } else { 0 } \
         }",
    );
}
