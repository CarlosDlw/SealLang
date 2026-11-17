//! Lowering of struct, range, and array patterns.

use seal_def::DefKind;

use super::support::{Fixture, function, lower_clean, tail};
use crate::pattern::PatternKind;
use crate::{Body, ExprKind, Hir};

fn match_arms<'a>(hir: &'a Hir, body: &'a Body) -> &'a [crate::pattern::MatchArm] {
    match &hir.expr(tail(hir, body)).kind {
        ExprKind::Match { arms, .. } => arms,
        other => panic!("expected a match expression, got {other:?}"),
    }
}

#[test]
fn struct_fields_are_resolved_into_declaration_order() {
    let (fixture, hir) = lower_clean(
        "struct Point { x: i32, y: i32 } \
         func total(point: Point, values: [i32; 2]) -> i32 { \
           match point { \
             Point { y: 0..=9, x } => { \
               match values { [first, second] => first + second + x } \
             } \
             Point { x, y } => x + y, \
           } \
         }",
    );
    let (owner, body) = function(&fixture, &hir, "total");
    let arms = match_arms(&hir, body);

    let PatternKind::Struct { adt, fields } = &hir.pattern(arms[0].pattern).kind else {
        panic!("expected a struct pattern");
    };
    assert_eq!(fixture.symbol(hir.definitions().name(*adt)), "Point");
    assert_eq!(fields.len(), 2);
    assert_eq!(fixture.symbol(hir.definitions().name(fields[0].0)), "x");
    assert_eq!(fixture.symbol(hir.definitions().name(fields[1].0)), "y");
    assert!(matches!(
        &hir.pattern(fields[1].1).kind,
        PatternKind::Range {
            inclusive: true,
            ..
        }
    ));

    let PatternKind::Binding { def, .. } = &hir.pattern(fields[0].1).kind else {
        panic!("expected the shorthand field to bind");
    };
    assert_eq!(hir.definitions().get(*def).kind, DefKind::Binding);
    assert_eq!(hir.definitions().get(*def).parent, Some(owner));

    let arm_body = hir.expr(arms[0].body);
    let ExprKind::Block(block) = arm_body.kind else {
        panic!("expected a block arm body");
    };
    let inner = hir
        .block(block)
        .tail
        .expect("the block has a tail expression");
    let ExprKind::Match {
        arms: inner_arms, ..
    } = &hir.expr(inner).kind
    else {
        panic!("expected a nested match");
    };
    let PatternKind::Array { prefix, .. } = &hir.pattern(inner_arms[0].pattern).kind else {
        panic!("expected an array pattern");
    };
    assert_eq!(prefix.len(), 2);
}

#[test]
fn a_rest_pattern_leaves_the_remaining_fields_unmatched() {
    let (fixture, hir) = lower_clean(
        "struct Point { x: i32, y: i32 } \
         func main(point: Point) -> i32 { match point { Point { x, .. } => x } }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let arms = match_arms(&hir, body);
    let PatternKind::Struct { fields, .. } = &hir.pattern(arms[0].pattern).kind else {
        panic!("expected a struct pattern");
    };
    assert_eq!(fields.len(), 1);
    assert_eq!(fixture.symbol(hir.definitions().name(fields[0].0)), "x");
}

#[test]
fn struct_pattern_field_errors_are_reported() {
    let fixture = Fixture::new(
        "struct Point { x: i32 } \
         func main(point: Point) -> i32 { match point { Point { y } => 0 } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2046", "has no field");

    let fixture = Fixture::new(
        "struct Point { x: i32 } \
         func main(point: Point) -> i32 { match point { Point { x, x } => 0 } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2047", "matched twice");

    let fixture = Fixture::new(
        "struct Point { x: i32, y: i32 } \
         func main(point: Point) -> i32 { match point { Point { x } => 0 } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2048", "missing");

    let fixture = Fixture::new(
        "enum Mode { On } \
         func main(mode: Mode) -> i32 { match mode { Mode { value } => 0 } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2042", "is an enum, not a struct");
}
