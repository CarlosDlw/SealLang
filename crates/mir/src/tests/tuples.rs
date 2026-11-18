//! Tuple construction, positional projections, and pattern lowering.

use crate::body::Body;
use crate::operand::Operand;
use crate::place::Projection;
use crate::rvalue::Rvalue;

use super::support::{assignment, function, lower_clean, ty_text};

#[test]
fn tuple_literals_keep_elements_in_order_and_their_type() {
    let (fixture, hir, mir) = lower_clean("func main() -> i32 { let pair = (1, true); pair.0 }");
    let body = function(&fixture, &hir, &mir, "main");
    let mut tuple = None;
    let mut read = None;
    for block in body.blocks.iter() {
        for statement in &block.statements {
            let (place, rvalue) = assignment(statement);
            if matches!(rvalue, Rvalue::Tuple { .. }) {
                tuple = Some(rvalue);
            }
            if place.expect_local() == Body::return_local() {
                read = Some(rvalue);
            }
        }
    }
    let Some(Rvalue::Tuple { elements, ty }) = tuple else {
        panic!("expected a tuple rvalue");
    };
    assert_eq!(elements.len(), 2);
    assert_eq!(ty_text(&hir, *ty), "(i32, bool)");
    let Some(Rvalue::Use(Operand::Copy(place))) = read else {
        panic!("expected a read of the tuple field");
    };
    assert!(matches!(place.projections(), [Projection::TupleField(0)]));
}

#[test]
fn tuple_pattern_bindings_project_each_element() {
    let (fixture, hir, mir) = lower_clean(
        "func main(pair: (i32, i32)) -> i32 { \
           match pair { (left, right) => left + right } \
         }",
    );
    let body = function(&fixture, &hir, &mir, "main");
    let mut projected = Vec::new();
    for block in body.blocks.iter() {
        for statement in &block.statements {
            let (_, rvalue) = assignment(statement);
            if let Rvalue::Use(Operand::Copy(place)) = rvalue {
                if let Some(Projection::TupleField(index)) = place.projections().last() {
                    projected.push(*index);
                }
            }
        }
    }
    projected.sort_unstable();
    assert_eq!(projected, [0, 1], "one read per bound tuple element");
}
