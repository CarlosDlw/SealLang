//! Lowering `for` loops to MIR.

use crate::place::Projection;
use crate::rvalue::{BinOp, Rvalue};
use crate::terminator::TerminatorKind;

use super::support::{assignment, function, lower_clean};

#[test]
fn a_for_range_lowers_to_a_condition_body_and_increment() {
    let (fixture, hir, mir) = lower_clean(
        "func main(limit: i32) -> i32 { \
           let mut total = 0; \
           for value in 0..limit { total += value; } \
           total \
         }",
    );
    let body = function(&fixture, &hir, &mir, "main");
    let mut has_less = false;
    let mut has_add = false;
    let mut switches = 0;
    for block in body.blocks.iter() {
        if matches!(block.terminator.kind, TerminatorKind::SwitchInt { .. }) {
            switches += 1;
        }
        for statement in &block.statements {
            match assignment(statement).1 {
                Rvalue::Binary { op: BinOp::Lt, .. } => has_less = true,
                Rvalue::Binary { op: BinOp::Add, .. } => has_add = true,
                _ => {}
            }
        }
    }
    assert!(has_less, "the range test compares the counter with the end");
    assert!(has_add, "the increment block adds one to the counter");
    assert!(switches >= 1, "the loop tests the counter before each body");
}

#[test]
fn sequence_loops_index_the_collection_and_borrow_elements() {
    let (fixture, hir, mir) = lower_clean(
        "func main(values: *[i32]) -> i32 { \
           let mut total = 0; \
           for value in values { total += *value; } \
           total \
         }",
    );
    let body = function(&fixture, &hir, &mir, "main");
    let mut has_index = false;
    let mut has_ref = false;
    let mut has_len = false;
    for block in body.blocks.iter() {
        for statement in &block.statements {
            match assignment(statement).1 {
                Rvalue::Ref { place, .. } => {
                    has_ref = true;
                    if place
                        .projections()
                        .iter()
                        .any(|projection| matches!(projection, Projection::Index { .. }))
                    {
                        has_index = true;
                    }
                }
                Rvalue::Len { .. } => has_len = true,
                Rvalue::Use(crate::operand::Operand::Copy(place))
                    if place
                        .projections()
                        .iter()
                        .any(|projection| matches!(projection, Projection::Index { .. })) =>
                {
                    has_index = true;
                }
                _ => {}
            }
        }
    }
    assert!(has_index, "an element is read by runtime index");
    assert!(has_ref, "each element is reborrowed for the pattern");
    assert!(has_len, "a slice source reads its length once");
}
