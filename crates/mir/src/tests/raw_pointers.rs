//! Raw pointer projection lowering.

use super::support::{function, lower_clean};
use crate::operand::Operand;
use crate::place::{Place, Projection};
use crate::rvalue::{IntrinsicReceiver, Rvalue};
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
fn a_raw_pointer_method_receives_the_pointer_by_value() {
    let (fixture, hir, mir) =
        lower_clean("func main(pointer: *raw i32) -> bool { pointer.is_null() }");
    let body = function(&fixture, &hir, &mir, "main");
    let mut found = false;
    for statement in body.blocks.iter().flat_map(|block| block.statements.iter()) {
        if let StatementKind::Assign {
            rvalue:
                Rvalue::Intrinsic {
                    receiver: IntrinsicReceiver::Value(_),
                    ..
                },
            ..
        } = &statement.kind
        {
            found = true;
        }
    }
    assert!(found, "the pointer method receives a value, not a place");
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

#[test]
fn a_write_through_a_raw_pointer_does_not_drop_a_destructor_value() {
    let (fixture, hir, mir) = lower_clean(
        "struct Guard { value: i32 } \
         impl Drop for Guard { func drop(self: *mut Guard) { } } \
         func main(pointer: *raw mut Guard) { unsafe { *pointer = Guard { value: 1 }; } }",
    );
    let body = function(&fixture, &hir, &mir, "main");
    let mut wrote = false;
    for statement in body.blocks.iter().flat_map(|block| block.statements.iter()) {
        match &statement.kind {
            StatementKind::Assign { place, .. } if has_raw_deref(place) => wrote = true,
            StatementKind::Drop { place, .. } if has_raw_deref(place) => {
                panic!("a raw write must not drop the old value")
            }
            _ => {}
        }
    }
    assert!(wrote, "the assignment targets the raw dereference");
}
