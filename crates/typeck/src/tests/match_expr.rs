//! Type checking of `match`: result types, bindings, guards, and coverage.

use seal_hir::{ExprKind, PatternKind};

use super::support::{Fixture, check_clean, expr_ty, function, tail, ty_text_with};

#[test]
fn a_match_expression_has_the_type_of_its_arms() {
    let (fixture, hir, results) = check_clean(
        "enum Mode { On, Off } \
         func main() -> i32 { match Mode::On { Mode::On => 1, Mode::Off => 0 } }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    assert_eq!(
        ty_text_with(&fixture, &hir, expr_ty(&results, tail(&hir, body))),
        "i32"
    );
}

#[test]
fn pattern_bindings_receive_the_field_type() {
    let (fixture, hir, results) = check_clean(
        "enum Command { Stop, Move(i32) } \
         func classify(command: Command) -> i32 { \
           match command { Command::Move(step) => step + 1, Command::Stop => 0 } \
         }",
    );
    let (_, body) = function(&fixture, &hir, "classify");
    let ExprKind::Match { arms, .. } = &hir.expr(tail(&hir, body)).kind else {
        panic!("expected a match expression");
    };
    let PatternKind::Variant { fields, .. } = &hir.pattern(arms[0].pattern).kind else {
        panic!("expected a variant pattern");
    };
    let PatternKind::Binding { def, .. } = &hir.pattern(fields[0].1).kind else {
        panic!("expected a binding pattern");
    };
    let ty = results.local_ty(*def).expect("the binding has a type");
    assert_eq!(ty_text_with(&fixture, &hir, ty), "i32");
}

#[test]
fn every_arm_must_produce_the_result_type() {
    let fixture = Fixture::new("func main() -> i32 { match 1 { 0 => true, _ => 0 } }");
    let _ = fixture.check(64);
    fixture.expect("E3001", "mismatched types");
}

#[test]
fn a_match_needs_every_value_of_a_finite_type() {
    let (fixture, _, _) =
        check_clean("func main() -> i32 { match true { true => 1, false => 0 } }");
    fixture.assert_clean();

    let fixture = Fixture::new(
        "enum Mode { On, Off } func main() -> i32 { match Mode::On { Mode::On => 1 } }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3032", "Mode::Off");

    let fixture = Fixture::new("func main() -> i32 { match 1 { 0 => 0 } }");
    let _ = fixture.check(64);
    fixture.expect("E3032", "non-exhaustive match");

    let (_, _, _) = check_clean("func main() -> i32 { match 1 { 0 => 0, _ => 1 } }");
}

#[test]
fn an_alternative_list_counts_every_alternative_as_coverage() {
    let (_, _, _) = check_clean("func main() -> i32 { match true { true | false => 1 } }");
}

#[test]
fn an_arm_after_a_covering_arm_is_unreachable() {
    let fixture = Fixture::new("func main() -> i32 { match true { _ => 0, true => 1 } }");
    let _ = fixture.check(64);
    fixture.expect("E3033", "unreachable");

    let fixture = Fixture::new(
        "enum Mode { On, Off } \
         func main() -> i32 { match Mode::On { Mode::On => 1, Mode::Off => 0, Mode::On => 2 } }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3033", "unreachable");
}

#[test]
fn a_guard_must_be_boolean() {
    let fixture = Fixture::new("func main() -> i32 { match 1 { value if value => 0, _ => 1 } }");
    let _ = fixture.check(64);
    assert!(
        fixture.codes().iter().any(|code| code == "E3003"),
        "expected E3003, got {:?}",
        fixture.codes()
    );
}

#[test]
fn a_guard_may_read_copy_bindings() {
    let (_, _, _) =
        check_clean("func main() -> i32 { match 1 { value if value > 0 => 0, _ => 1 } }");
}

#[test]
fn a_guard_cannot_read_a_non_copy_binding() {
    let fixture = Fixture::new(
        "struct Token { value: i32 } \
         enum Holder { Empty, Full(Token) } \
         func read(holder: Holder) -> i32 { \
           match holder { Holder::Full(token) if token.value > 0 => 1, _ => 0 } \
         }",
    );
    let _ = fixture.check(64);
    assert!(
        fixture.codes().iter().any(|code| code == "E2045"),
        "expected E2045, got {:?}",
        fixture.codes()
    );
}

#[test]
fn a_guarded_arm_does_not_make_a_match_exhaustive() {
    let fixture = Fixture::new(
        "enum Mode { On, Off } \
         func main() -> i32 { match Mode::On { Mode::On if true => 1, Mode::Off => 0 } }",
    );
    let _ = fixture.check(64);
    fixture.expect("E3032", "Mode::On");
}
