//! Type checking and coverage of struct, range, array, and `@` patterns.

use seal_hir::{ExprKind, PatternKind};

use super::support::{Fixture, check_clean, function, tail, ty_text_with};

#[test]
fn adjacent_ranges_cover_a_scalar_domain() {
    let (_, _, _) = check_clean(
        "func main(value: i32) -> i32 { match value { 0..=9 => 1, 10..=19 => 2, _ => 0 } }",
    );
    let (_, _, _) = check_clean("func main(value: u8) -> i32 { match value { 0..=255 => 1 } }");
    let (_, _, _) =
        check_clean("func main(letter: char) -> i32 { match letter { 'a'..='z' => 1, _ => 0 } }");
}

#[test]
fn a_gap_between_ranges_is_reported_with_a_witness() {
    let fixture =
        Fixture::new("func main(value: u8) -> i32 { match value { 0..=9 => 1, 20..=255 => 2 } }");
    let _ = fixture.check(64);
    fixture.expect("E3032", "10..=19");
}

#[test]
fn overlapping_ranges_make_later_arms_unreachable() {
    let fixture = Fixture::new(
        "func main(value: i32) -> i32 { match value { 0..=19 => 1, 10..=12 => 2, _ => 0 } }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3033", "unreachable");
}

#[test]
fn an_empty_range_is_rejected() {
    let fixture =
        Fixture::new("func main(value: i32) -> i32 { match value { 9..=0 => 1, _ => 0 } }");
    let _ = fixture.check(64);
    fixture.expect("E2050", "empty");
}

#[test]
fn an_array_pattern_must_match_the_length() {
    let fixture = Fixture::new(
        "func main(values: [i32; 3]) -> i32 { match values { [a, b] => a + b, _ => 0 } }",
    );
    let _ = fixture.check(64);
    fixture.expect("E2049", "element");

    let (_, _, _) = check_clean(
        "func main(values: [i32; 3]) -> i32 { \
           match values { [_, _, 0] => 1, [_, _, _] => 0 } \
         }",
    );
    let (_, _, _) = check_clean(
        "func main(values: [i32; 3]) -> i32 { \
           match values { [first, .., last] => first + last } \
         }",
    );
}

#[test]
fn a_struct_field_pattern_is_checked_against_the_field_type() {
    let fixture = Fixture::new(
        "struct Point { x: i32 } \
         func main(point: Point) -> i32 { \
           match point { Point { x: true } => 0, Point { x: _ } => 1 } \
         }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn a_binding_at_pattern_keeps_the_scrutinee_type() {
    let (fixture, hir, results) =
        check_clean("func main(value: i32) -> i32 { match value { n @ 1..=9 => n, _ => 0 } }");
    let (_, body) = function(&fixture, &hir, "main");
    let ExprKind::Match { arms, .. } = &hir.expr(tail(&hir, body)).kind else {
        panic!("expected a match expression");
    };
    let PatternKind::Binding { def, .. } = &hir.pattern(arms[0].pattern).kind else {
        panic!("expected an @ binding");
    };
    let ty = results.local_ty(*def).expect("the binding has a type");
    assert_eq!(ty_text_with(&fixture, &hir, ty), "i32");
}
