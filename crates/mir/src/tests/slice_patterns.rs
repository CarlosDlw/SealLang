//! Slice pattern lowering: runtime length tests and index projections.

use crate::operand::Operand;
use crate::place::Projection;
use crate::rvalue::{BinOp, Rvalue};
use crate::terminator::TerminatorKind;

use super::support::{assignment, function, lower_clean};

#[test]
fn a_slice_pattern_reads_and_tests_the_runtime_length() {
    let (fixture, hir, mir) = lower_clean(
        "func f(values: *[i32]) -> i32 { match values { [] => 0, [first, ..] => first } }",
    );
    let body = function(&fixture, &hir, &mir, "f");
    let rvalues: Vec<&Rvalue> = body
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .map(|statement| assignment(statement).1)
        .collect();

    assert!(
        rvalues
            .iter()
            .any(|rvalue| matches!(rvalue, Rvalue::Len { .. })),
        "the pattern reads the slice length"
    );
    assert!(
        rvalues
            .iter()
            .any(|rvalue| matches!(rvalue, Rvalue::Binary { op: BinOp::Ge, .. })),
        "a rest pattern tests that the length is large enough"
    );
    assert!(
        body.blocks
            .iter()
            .any(|block| matches!(&block.terminator.kind, TerminatorKind::SwitchInt { .. })),
        "the exact-length arm switches on the length"
    );
}

#[test]
fn a_slice_suffix_is_indexed_from_the_end() {
    let (fixture, hir, mir) = lower_clean(
        "func f(values: *[i32]) -> i32 { match values { [.., last] => last, [] => 0 } }",
    );
    let body = function(&fixture, &hir, &mir, "f");
    let mut dynamic = false;
    let mut subtracts = false;
    for block in body.blocks.iter() {
        for statement in &block.statements {
            let (_, rvalue) = assignment(statement);
            match rvalue {
                Rvalue::Use(Operand::Copy(place)) => {
                    if place.projections.iter().any(|projection| {
                        matches!(projection, Projection::Index { constant: None, .. })
                    }) {
                        dynamic = true;
                    }
                }
                Rvalue::Binary { op: BinOp::Sub, .. } => subtracts = true,
                _ => {}
            }
        }
    }

    assert!(dynamic, "the last element uses a runtime index");
    assert!(subtracts, "the index is measured from the length");
}
