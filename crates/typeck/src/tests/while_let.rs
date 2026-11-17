//! Type checking `while let` statements.

use super::support::{Fixture, check_clean};

#[test]
fn while_let_accepts_variant_range_and_slice_patterns() {
    let (_, _, _) = check_clean(
        "enum Option { None, Some(i32) } \
         func next() -> Option { Option::None } \
         func main() -> i32 { \
           let mut total = 0; \
           while let Option::Some(value) = next() { total += value; } \
           total \
         }",
    );

    let (_, _, _) = check_clean(
        "func main() -> i32 { \
           let mut value = 0; \
           let mut total = 0; \
           while let 0..=3 = value { \
             value += 1; \
             if value == 2 { continue; } \
             if value > 3 { break; } \
             total += value; \
           } \
           total \
         }",
    );

    let (_, _, _) = check_clean(
        "func sum(values: *[i32]) -> i32 { \
           let mut rest = values; \
           let mut total = 0; \
           while let [first, ..] = rest { total += first; rest = &rest[1..]; } \
           total \
         }",
    );
}

#[test]
fn a_pattern_that_does_not_match_the_scrutinee_type_is_rejected() {
    let fixture = Fixture::new(
        "enum Option { None, Some(i32) } \
         func main() { while let Option::Some(value) = 0 { } }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3001", "mismatched types");
}
