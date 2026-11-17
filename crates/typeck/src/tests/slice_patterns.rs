//! Slice patterns: length coverage, bindings, and move restrictions.

use super::support::{Fixture, check_clean};

#[test]
fn slice_lengths_can_be_covered_by_three_arms() {
    let (_, _, _) = check_clean(
        "func main(values: *[i32]) -> i32 { \
           match values { [] => 0, [only] => only, [first, .., last] => first + last } \
         }",
    );
    let (_, _, _) = check_clean(
        "func main(values: *mut [i32]) -> i32 { \
           match values { [..] => 1 } \
         }",
    );
}

#[test]
fn a_missing_slice_length_is_reported_with_a_witness() {
    let fixture =
        Fixture::new("func main(values: *[i32]) -> i32 { match values { [first, ..] => first } }");
    let _ = fixture.check(64);
    fixture.expect("E3032", "the slice [] is not covered");
}

#[test]
fn an_arm_after_a_covering_slice_pattern_is_unreachable() {
    let fixture =
        Fixture::new("func main(values: *[i32]) -> i32 { match values { [..] => 0, [] => 1 } }");
    let _ = fixture.check(64);
    fixture.expect("E3033", "unreachable");
}

#[test]
fn exact_lengths_and_suffixes_are_checked_per_length() {
    let fixture = Fixture::new(
        "func main(values: *[i32]) -> i32 { \
           match values { [] => 0, [only] => only, [left, right] => left + right } \
         }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3032", "the slice [_, _, _] is not covered");

    let (_, _, _) = check_clean(
        "func main(values: *[i32]) -> i32 { \
           match values { [] => 0, [only] => only, [left, right] => left + right, _ => 0 } \
         }",
    );
}

#[test]
fn a_non_copy_element_cannot_be_bound() {
    let fixture = Fixture::new(
        "struct Token { value: i32 } \
         func first(tokens: *[Token]) -> i32 { \
           match tokens { [token, ..] => token.value, _ => 0 } \
         }",
    );
    let _ = fixture.check(64);
    fixture.expect(
        "E3018",
        "cannot move a `Token` value out of a slice pattern",
    );

    let (_, _, _) = check_clean(
        "struct Token { value: i32 } \
         func first(tokens: *[Token]) -> i32 { \
           match tokens { [Token { value: 0 }, ..] => 1, _ => 0 } \
         }",
    );
}

#[test]
fn a_guard_may_read_a_copy_slice_binding() {
    let (_, _, _) = check_clean(
        "func main(values: *[i32]) -> i32 { \
           match values { [first, ..] if first > 0 => first, _ => 0 } \
         }",
    );
}

#[test]
fn an_element_pattern_is_checked_against_the_element_type() {
    let fixture =
        Fixture::new("func main(values: *[i32]) -> i32 { match values { ['a'] => 0, _ => 0 } }");
    let _ = fixture.check(64);
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn slice_fields_inside_a_struct_are_covered_by_length() {
    let (_, _, _) = check_clean(
        "struct Holder { values: *[i32] } \
         func main(holder: Holder) -> i32 { \
           match holder { \
             Holder { values: [] } => 0, \
             Holder { values: [only] } => only, \
             Holder { values: [first, .., last] } => first + last, \
           } \
         }",
    );

    let fixture = Fixture::new(
        "struct Holder { values: *[i32] } \
         func main(holder: Holder) -> i32 { \
           match holder { Holder { values: [..] } => 0, Holder { values: [] } => 1 } \
         }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3033", "unreachable");
}

#[test]
fn an_array_pattern_on_another_pointer_is_rejected() {
    let fixture = Fixture::new(
        "struct Point { x: i32 } \
         func main(point: *Point) -> i32 { match point { [value] => value } }",
    );
    let _ = fixture.check(64);
    fixture.expect("E2049", "an array pattern needs an array");
}
