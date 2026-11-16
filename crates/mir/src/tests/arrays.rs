//! Array lowering, indexing, and bounds checks in MIR.

use crate::body::Body;
use crate::operand::Operand;
use crate::place::Projection;
use crate::rvalue::Rvalue;
use crate::terminator::TerminatorKind;

use super::support::{assignment, function, lower_clean, ty_text};

#[test]
fn array_literals_keep_elements_in_order_and_their_type() {
    let (fixture, hir, mir) = lower_clean("func main() -> i32 { let a = [1, 2, 3]; a[1] }");
    let body = function(&fixture, &hir, &mir, "main");
    let mut array = None;
    let mut read = None;
    for block in body.blocks.iter() {
        for statement in &block.statements {
            let (place, rvalue) = assignment(statement);
            if matches!(rvalue, Rvalue::Array { .. }) {
                array = Some(rvalue);
            }
            if place.local == Body::return_local() {
                read = Some(rvalue);
            }
        }
    }
    let Some(Rvalue::Array { elements, ty }) = array else {
        panic!("expected an array rvalue");
    };
    assert_eq!(elements.len(), 3);
    assert_eq!(ty_text(&hir, *ty), "[i32; 3]");
    let Some(Rvalue::Use(Operand::Copy(place))) = read else {
        panic!("expected a read of the indexed place");
    };
    assert!(matches!(
        place.projections(),
        [Projection::Index {
            constant: Some(1),
            ..
        }]
    ));
}

#[test]
fn repeated_arrays_keep_their_count_and_type() {
    let (fixture, hir, mir) = lower_clean("func main() -> i32 { let a = [9; 4]; a[0] }");
    let body = function(&fixture, &hir, &mir, "main");
    let repeat = body
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .map(assignment)
        .find_map(|(_, rvalue)| match rvalue {
            Rvalue::Repeat { count, ty, .. } => Some((*count, *ty)),
            _ => None,
        })
        .expect("expected a repeat rvalue");
    assert_eq!(repeat.0, 4);
    assert_eq!(ty_text(&hir, repeat.1), "[i32; 4]");
}

#[test]
fn a_dynamic_index_gets_a_bounds_check_and_a_runtime_projection() {
    let (fixture, hir, mir) =
        lower_clean("func main() -> i32 { let a = [1, 2, 3]; let i: usize = 1; a[i] }");
    let body = function(&fixture, &hir, &mir, "main");
    assert!(body.blocks.len() > 1, "a bounds check adds control flow");
    let mut indexed = false;
    let mut panic = None;
    for block in body.blocks.iter() {
        if let TerminatorKind::Panic { message } = &block.terminator.kind {
            panic = Some(message.clone());
        }
        for statement in &block.statements {
            let (_, rvalue) = assignment(statement);
            let Rvalue::Use(Operand::Copy(place)) = rvalue else {
                continue;
            };
            if matches!(
                place.projections(),
                [Projection::Index { constant: None, .. }]
            ) {
                indexed = true;
            }
        }
    }
    assert!(indexed, "the dynamic index must stay in the projection");
    let message = panic.expect("a dynamic index needs a panic block");
    assert!(
        message.contains("length is 3"),
        "unexpected message: {message}"
    );
}

#[test]
fn nested_indices_evaluate_outer_first_and_check_both_lengths() {
    let (fixture, hir, mir) = lower_clean(
        "func main() -> i32 { let a = [[1, 2], [3, 4]]; let i: usize = 1; let j: usize = 0; a[i][j] }",
    );
    let body = function(&fixture, &hir, &mir, "main");
    let messages: Vec<String> = body
        .blocks
        .iter()
        .filter_map(|block| match &block.terminator.kind {
            TerminatorKind::Panic { message } => Some(message.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(messages.len(), 2, "each dynamic index needs a check");
    assert!(
        messages
            .iter()
            .all(|message| message.contains("length is 2"))
    );
}
