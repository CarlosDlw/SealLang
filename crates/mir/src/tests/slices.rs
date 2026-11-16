//! Slice reference construction, range checks, length reads, and indexing.

use crate::operand::Operand;
use crate::place::Projection;
use crate::rvalue::Rvalue;
use crate::terminator::TerminatorKind;

use super::support::{assignment, function, lower_clean, ty_text};

#[test]
fn a_range_builds_a_fat_reference_and_checks_both_bounds() {
    let (fixture, hir, mir) = lower_clean("func f(values: [i32; 4]) -> *[i32] { &values[1..3] }");
    let body = function(&fixture, &hir, &mir, "f");
    let slice = body
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .map(assignment)
        .find_map(|(_, rvalue)| match rvalue {
            Rvalue::SliceRef { start, end, ty, .. } => Some((start.is_some(), end.is_some(), *ty)),
            _ => None,
        })
        .expect("expected a slice reference");
    assert!(slice.0);
    assert!(slice.1);
    assert_eq!(ty_text(&hir, slice.2), "*[i32]");

    let panics = body
        .blocks
        .iter()
        .filter(|block| matches!(block.terminator.kind, TerminatorKind::Panic { .. }))
        .count();
    assert_eq!(panics, 2, "a bounded range checks order and the end");
}

#[test]
fn a_full_range_needs_no_runtime_check() {
    let (fixture, hir, mir) = lower_clean("func f(values: [i32; 4]) -> *[i32] { &values[..] }");
    let body = function(&fixture, &hir, &mir, "f");
    assert!(
        body.blocks
            .iter()
            .all(|block| !matches!(block.terminator.kind, TerminatorKind::Panic { .. }))
    );
}

#[test]
fn len_lowers_to_a_length_read() {
    let (fixture, hir, mir) = lower_clean(
        "func f(values: [i32; 4]) -> usize { let slice: *[i32] = &values; len(slice) }",
    );
    let body = function(&fixture, &hir, &mir, "f");
    assert!(
        body.blocks
            .iter()
            .flat_map(|block| &block.statements)
            .any(|statement| matches!(assignment(statement).1, Rvalue::Len { .. }))
    );
}

#[test]
fn slice_indexing_uses_the_fat_pointer_and_a_runtime_length() {
    let (fixture, hir, mir) =
        lower_clean("func f(values: *[i32], index: usize) -> i32 { values[index] }");
    let body = function(&fixture, &hir, &mir, "f");
    let mut indexed = false;
    for block in body.blocks.iter() {
        for statement in &block.statements {
            let (_, rvalue) = assignment(statement);
            let Rvalue::Use(Operand::Copy(place)) = rvalue else {
                continue;
            };
            if matches!(
                place.projections(),
                [Projection::Deref, Projection::Index { constant: None, .. }]
            ) {
                indexed = true;
            }
        }
    }
    assert!(indexed, "slice indexing keeps deref then index in MIR");
    assert!(
        body.blocks
            .iter()
            .flat_map(|block| &block.statements)
            .any(|statement| matches!(assignment(statement).1, Rvalue::Len { .. })),
        "the bounds check reads the runtime length"
    );
}
