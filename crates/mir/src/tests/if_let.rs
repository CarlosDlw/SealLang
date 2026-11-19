//! Lowering `if let` expressions to MIR.

use super::support::{function, local_named, lower_clean};
use crate::operand::{Constant, Operand};
use crate::rvalue::Rvalue;
use crate::terminator::TerminatorKind;

#[test]
fn an_if_let_tests_the_pattern_and_binds_on_success() {
    let (fixture, hir, mir) = lower_clean(
        "enum Option { None, Some(i32) } \
         func main(value: Option) -> i32 { \
           if let Option::Some(inner) = value { inner } else { 0 } \
         }",
    );
    let body = function(&fixture, &hir, &mir, "main");
    local_named(&fixture, body, "inner");

    let mut discriminants = 0;
    let mut switches = 0;
    for block in body.blocks.iter() {
        if matches!(&block.terminator.kind, TerminatorKind::SwitchInt { .. }) {
            switches += 1;
        }
        discriminants += block
            .statements
            .iter()
            .filter(|statement| {
                matches!(
                    statement.kind,
                    crate::statement::StatementKind::Assign {
                        rvalue: Rvalue::Discriminant(_),
                        ..
                    }
                )
            })
            .count();
    }
    assert_eq!(discriminants, 1, "the variant test reads the tag once");
    assert!(switches >= 1, "the tag test branches to the else path");
}

#[test]
fn a_slice_pattern_tests_the_runtime_length_in_if_let() {
    let (fixture, hir, mir) = lower_clean(
        "func main(values: *[i32]) -> i32 { \
           if let [first, ..] = values { first } else { 0 } \
         }",
    );
    let body = function(&fixture, &hir, &mir, "main");
    assert!(
        body.blocks.iter().any(|block| matches!(
            &block.terminator.kind,
            TerminatorKind::SwitchInt { targets, .. }
                if targets.iter().any(|target| target.value == 1)
        )),
        "the length test branches to the matching slice"
    );
}

#[test]
fn an_if_let_in_tail_position_drops_its_scrutinee() {
    // The `if let` is the block's tail, so the temporary that holds the
    // scrutinee ends with the block. A value it still owns must be dropped
    // there rather than leaked.
    let source = "struct Token { tag: i32 } \
        impl Drop for Token { \
            func drop(self: *mut Token) { unsafe { COUNT = COUNT + 1; } } \
        } \
        static mut COUNT: i32 = 0; \
        enum Slot { Value(Token), Empty } \
        func release() { if let Slot::Empty = Slot::Value(Token { tag: 1 }) {} }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "release");
    let drops = body
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .filter(|statement| matches!(statement.kind, crate::statement::StatementKind::Drop { .. }))
        .count();
    assert!(drops >= 1, "the scrutinee temporary must have a drop site");
}

#[test]
fn if_let_without_else_assigns_unit_on_failure() {
    let (fixture, hir, mir) = lower_clean("func main(value: i32) { if let 0 = value { } }");
    let body = function(&fixture, &hir, &mir, "main");
    let units = body
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .filter(|statement| {
            matches!(
                statement.kind,
                crate::statement::StatementKind::Assign {
                    rvalue: Rvalue::Use(Operand::Constant(Constant::Unit)),
                    ..
                }
            )
        })
        .count();
    assert!(units >= 1, "the missing else produces unit");
}
