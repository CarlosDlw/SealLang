//! Raw pointer projection lowering.

use super::support::{function, lower_clean};
use crate::operand::Operand;
use crate::place::{Place, Projection};
use crate::rvalue::Rvalue;
use crate::statement::StatementKind;

fn has_raw_deref(place: &Place) -> bool {
    place.projections().contains(&Projection::RawDeref)
}

#[test]
fn a_raw_dereference_lowers_to_a_raw_projection() {
    let (fixture, hir, mir) =
        lower_clean("func main(pointer: *raw i32) -> i32 { unsafe { *pointer } }");
    let body = function(&fixture, &hir, &mir, "main");
    let mut found = false;
    for statement in body.blocks.iter().flat_map(|block| block.statements.iter()) {
        if let StatementKind::Assign {
            rvalue: Rvalue::Use(Operand::Copy(place) | Operand::Move(place)),
            ..
        } = &statement.kind
        {
            found |= has_raw_deref(place);
        }
    }
    assert!(found, "the read lowers through a raw dereference");
}

#[test]
fn a_write_through_a_raw_pointer_targets_the_raw_place_without_dropping() {
    let (fixture, hir, mir) =
        lower_clean("func main(pointer: *raw mut i32) { unsafe { *pointer = 1; } }");
    let body = function(&fixture, &hir, &mir, "main");
    let mut wrote = false;
    for statement in body.blocks.iter().flat_map(|block| block.statements.iter()) {
        match &statement.kind {
            StatementKind::Assign { place, .. } if has_raw_deref(place) => wrote = true,
            StatementKind::Drop { place, .. } if has_raw_deref(place) => {
                panic!("a raw write must not drop the previous value")
            }
            _ => {}
        }
    }
    assert!(wrote, "the assignment targets the raw dereference");
}

#[test]
fn a_non_copy_value_moves_out_of_a_raw_pointer() {
    let (fixture, hir, mir) = lower_clean(
        "struct Payload { value: i32 } \
         func main(pointer: *raw Payload) -> Payload { unsafe { *pointer } }",
    );
    let body = function(&fixture, &hir, &mir, "main");
    let mut moved = false;
    for statement in body.blocks.iter().flat_map(|block| block.statements.iter()) {
        if let StatementKind::Assign {
            rvalue: Rvalue::Use(Operand::Move(place)),
            ..
        } = &statement.kind
        {
            moved |= has_raw_deref(place);
        }
    }
    assert!(moved, "a non-Copy pointee is moved, not copied");
}
