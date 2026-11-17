//! Lowering `while let` statements to MIR.

use super::support::{function, local_named, lower_clean};
use crate::rvalue::Rvalue;
use crate::terminator::TerminatorKind;

#[test]
fn a_while_let_tests_the_pattern_at_the_loop_head() {
    let (fixture, hir, mir) = lower_clean(
        "enum Option { None, Some(i32) } \
         func next() -> Option { Option::None } \
         func main() -> i32 { \
           let mut total = 0; \
           while let Option::Some(inner) = next() { total += inner; } \
           total \
         }",
    );
    let body = function(&fixture, &hir, &mir, "main");
    local_named(&fixture, body, "inner");

    let discriminants = body
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
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
    assert_eq!(discriminants, 1, "the variant test reads the tag once");
    assert!(
        body.blocks
            .iter()
            .any(|block| matches!(&block.terminator.kind, TerminatorKind::SwitchInt { .. })),
        "the tag test selects the body or the exit"
    );
}

#[test]
fn the_body_jumps_back_to_the_condition() {
    let (fixture, hir, mir) = lower_clean(
        "func main() -> i32 { \
           let mut value = 0; \
           while let 0..=2 = value { value += 1; } \
           value \
         }",
    );
    let body = function(&fixture, &hir, &mir, "main");
    let has_backedge = body.blocks.iter().any(|block| {
        let TerminatorKind::Goto(target) = &block.terminator.kind else {
            return false;
        };
        matches!(
            &body.block(*target).terminator.kind,
            TerminatorKind::SwitchInt { .. }
        )
    });
    assert!(has_backedge, "the body returns to the pattern test");
}
