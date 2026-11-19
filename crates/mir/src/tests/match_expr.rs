//! MIR lowering of `match` expressions.

use seal_ty::VariantIdx;

use super::support::{function, lower_clean};
use crate::body::Body;
use crate::operand::Operand;
use crate::place::Projection;
use crate::rvalue::{BinOp, Rvalue};
use crate::statement::StatementKind;
use crate::terminator::TerminatorKind;

fn rvalues(body: &Body) -> Vec<&Rvalue> {
    body.blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .map(|statement| match &statement.kind {
            StatementKind::Assign { rvalue, .. } => rvalue,
            other => panic!("expected an assignment, found {other:?}"),
        })
        .collect()
}

#[test]
fn a_variant_pattern_switches_on_the_discriminant() {
    let (fixture, hir, mir) = lower_clean(
        "enum Mode { On, Off } \
         func main(mode: Mode) -> i32 { match mode { Mode::On => 1, Mode::Off => 0 } }",
    );
    let body = function(&fixture, &hir, &mir, "main");
    let tags = rvalues(body)
        .into_iter()
        .filter(|rvalue| matches!(rvalue, Rvalue::Discriminant(_)))
        .count();
    assert_eq!(tags, 2, "each arm reads the tag once");

    let mut cases = Vec::new();
    for block in body.blocks.iter() {
        if let TerminatorKind::SwitchInt { targets, .. } = &block.terminator.kind {
            cases.extend(targets.iter().map(|target| target.value));
        }
    }
    assert!(cases.contains(&0), "`Off` has discriminant 0");
    assert!(cases.contains(&1), "`On` has discriminant 1");
}

#[test]
fn a_field_pattern_projects_into_the_variant_payload() {
    let (fixture, hir, mir) = lower_clean(
        "enum Command { Write { code: i32 }, Stop } \
         func main(command: Command) -> i32 { \
           match command { Command::Write { code } => code, Command::Stop => 0 } \
         }",
    );
    let body = function(&fixture, &hir, &mir, "main");
    let mut projected = Vec::new();
    for rvalue in rvalues(body) {
        if let Rvalue::Use(Operand::Copy(place)) = rvalue {
            if let Some(Projection::VariantField { variant, field }) = place.projections().last() {
                projected.push((*variant, *field));
            }
        }
    }
    assert_eq!(projected.len(), 1, "one binding reads one field");
    let (variant, field) = projected[0];
    assert_eq!(variant, VariantIdx::from_u32(0).expect("small index"));
    assert_eq!(fixture.symbol(hir.definitions().name(field)), "code");
}

#[test]
fn a_literal_pattern_compares_by_value() {
    let (fixture, hir, mir) =
        lower_clean("func main(value: i32) -> i32 { match value { 0 => 1, _ => 0 } }");
    let body = function(&fixture, &hir, &mir, "main");
    assert!(
        rvalues(body)
            .into_iter()
            .any(|rvalue| matches!(rvalue, Rvalue::Binary { op: BinOp::Eq, .. })),
        "a literal pattern lowers to an equality test"
    );
    assert!(
        body.blocks.iter().any(|block| matches!(
            &block.terminator.kind,
            TerminatorKind::SwitchInt { targets, .. }
                if targets.iter().any(|target| target.value == 1)
        )),
        "the equality result selects the arm"
    );
}

#[test]
fn alternatives_expand_into_separate_pattern_tests() {
    let (fixture, hir, mir) = lower_clean(
        "enum Mode { On, Off, Idle } \
         func main(mode: Mode) -> i32 { \
           match mode { Mode::On | Mode::Off => 1, Mode::Idle => 0 } \
         }",
    );
    let body = function(&fixture, &hir, &mir, "main");
    let tags = rvalues(body)
        .into_iter()
        .filter(|rvalue| matches!(rvalue, Rvalue::Discriminant(_)))
        .count();
    assert_eq!(tags, 3, "two alternatives plus the remaining arm");
}

#[test]
fn a_guard_runs_after_the_pattern_tests() {
    let (fixture, hir, mir) = lower_clean(
        "enum Mode { Stop, Move(i32) } \
         func main(mode: Mode) -> i32 { \
           match mode { \
             Mode::Move(step) if step > 0 => step, \
             Mode::Move(_) => 0, \
             Mode::Stop => 0, \
           } \
         }",
    );
    let body = function(&fixture, &hir, &mir, "main");
    assert!(
        rvalues(body)
            .into_iter()
            .any(|rvalue| matches!(rvalue, Rvalue::Binary { op: BinOp::Gt, .. })),
        "the guard compares the bound value"
    );

    let guard_block = body.blocks.iter().find(|block| {
        let TerminatorKind::SwitchInt { targets, .. } = &block.terminator.kind else {
            return false;
        };
        targets.iter().any(|target| {
            target.value == 1
                && body
                    .block(target.target)
                    .statements
                    .iter()
                    .any(|statement| {
                        matches!(
                            &statement.kind,
                            StatementKind::Assign {
                                rvalue: Rvalue::Binary { op: BinOp::Gt, .. },
                                ..
                            }
                        )
                    })
        })
    });
    assert!(
        guard_block.is_some(),
        "the successful tag test enters the guard comparison"
    );
}

#[test]
fn an_exhaustive_match_ends_its_arm_tests_in_an_unreachable_block() {
    let (fixture, hir, mir) = lower_clean(
        "enum Mode { On, Off } \
         func main(mode: Mode) -> i32 { match mode { Mode::On => 1, Mode::Off => 0 } }",
    );
    let body = function(&fixture, &hir, &mir, "main");
    assert!(
        body.blocks
            .iter()
            .any(|block| matches!(&block.terminator.kind, TerminatorKind::Unreachable)),
        "falling past the last arm is unreachable"
    );
}
