//! Pattern lowering and name resolution for `match`.

use seal_def::DefKind;
use seal_ty::VariantIdx;

use super::support::{Fixture, function, lower_clean, tail};
use crate::pattern::{MatchArm, PatternKind};
use crate::{Body, ExprKind, Hir, Res};

fn match_arms<'a>(hir: &'a Hir, body: &Body) -> &'a [MatchArm] {
    match &hir.expr(tail(hir, body)).kind {
        ExprKind::Match { arms, .. } => arms,
        other => panic!("expected a match expression, got {other:?}"),
    }
}

#[test]
fn named_patterns_are_reordered_into_declaration_order() {
    let (fixture, hir) = lower_clean(
        "enum Command { Write { code: i32, repeat: i32 }, Stop } \
         func main(command: Command) -> i32 { \
           match command { \
             Command::Write { repeat, code } => code + repeat, \
             Command::Stop => 0, \
           } \
         }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let arms = match_arms(&hir, body);
    assert_eq!(arms.len(), 2);
    let PatternKind::Variant {
        variant, fields, ..
    } = &hir.pattern(arms[0].pattern).kind
    else {
        panic!("expected a variant pattern");
    };
    assert_eq!(*variant, VariantIdx::from_u32(0).expect("small index"));
    assert_eq!(fields.len(), 2);
    assert_eq!(
        fixture.symbol(hir.definitions().name(fields[0].0)),
        "code",
        "fields follow the declaration, not the pattern"
    );
    assert_eq!(
        fixture.symbol(hir.definitions().name(fields[1].0)),
        "repeat"
    );
    assert!(matches!(
        &hir.pattern(arms[1].pattern).kind,
        PatternKind::Variant { fields, .. } if fields.is_empty()
    ));
}

#[test]
fn bindings_become_definitions_with_their_names() {
    let (fixture, hir) = lower_clean(
        "enum Command { Move(i32) } \
         func main(command: Command) -> i32 { \
           match command { Command::Move(step) => step } \
         }",
    );
    let (owner, body) = function(&fixture, &hir, "main");
    let arms = match_arms(&hir, body);
    let PatternKind::Variant { fields, .. } = &hir.pattern(arms[0].pattern).kind else {
        panic!("expected a variant pattern");
    };
    let (field, pattern) = fields[0];
    assert_eq!(fixture.symbol(hir.definitions().name(field)), "#0");
    let PatternKind::Binding { name, def } = &hir.pattern(pattern).kind else {
        panic!("expected a binding pattern");
    };
    assert_eq!(fixture.symbol(*name), "step");
    assert_eq!(hir.definitions().get(*def).kind, DefKind::Binding);
    assert_eq!(hir.definitions().get(*def).parent, Some(owner));

    let ExprKind::Path(Res::Local(resolved)) = &hir.expr(arms[0].body).kind else {
        panic!("expected the arm body to resolve to the binding");
    };
    assert_eq!(resolved, def);
}

#[test]
fn a_binding_repeated_in_one_pattern_is_rejected() {
    let fixture = Fixture::new(
        "enum Pair { Both(i32, i32) } \
         func main(pair: Pair) -> i32 { match pair { Pair::Both(value, value) => value } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2040", "bound twice");
}

#[test]
fn or_pattern_alternatives_must_bind_the_same_names() {
    let fixture = Fixture::new(
        "enum Option { Some(i32), None } \
         func main(option: Option) -> i32 { match option { Option::Some(value) | Option::None => value } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2041", "bind different names");
}

#[test]
fn variant_shape_errors_are_reported() {
    let fixture = Fixture::new(
        "enum Mode { On } func main(mode: Mode) -> i32 { match mode { Mode::On() => 0 } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2042", "does not have positional fields");

    let fixture = Fixture::new(
        "enum Mode { On } func main(mode: Mode) -> i32 { match mode { Mode::On { value } => 0 } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2042", "has no fields");

    let fixture = Fixture::new(
        "enum Pair { Both(i32, i32) } \
         func main(pair: Pair) -> i32 { match pair { Pair::Both(value) => value } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2043", "takes 2 field pattern(s)");

    let fixture = Fixture::new(
        "enum Mode { On } func main(mode: Mode) -> i32 { match mode { Mode::On(_) => 0 } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2042", "does not have positional fields");

    let fixture = Fixture::new(
        "enum Point { At { x: i32, y: i32 } } \
         func main(point: Point) -> i32 { match point { Point::At { x } => x } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2043", "is missing");

    let fixture = Fixture::new(
        "enum Point { At { x: i32 } } \
         func main(point: Point) -> i32 { match point { Point::At { z } => 0 } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2044", "has no field");

    let fixture = Fixture::new(
        "enum Point { At { x: i32 } } \
         func main(point: Point) -> i32 { match point { Point::At { x, x } => x } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2044", "matched twice");
}

#[test]
fn a_struct_cannot_be_matched_as_a_variant() {
    let fixture = Fixture::new(
        "struct Point { x: i32 } \
         func main(point: Point) -> i32 { match point { Point::At { x } => x } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2042", "is a struct, not an enum");
}

#[test]
fn unknown_enums_and_variants_are_reported() {
    let fixture = Fixture::new(
        "enum Mode { On } func main(mode: Mode) -> i32 { match mode { Missing::On => 0 } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2002", "cannot find type");

    let fixture = Fixture::new(
        "enum Mode { On } func main(mode: Mode) -> i32 { match mode { Mode::Off => 0 } }",
    );
    let _ = fixture.lower();
    fixture.expect("E2032", "has no variant");
}
