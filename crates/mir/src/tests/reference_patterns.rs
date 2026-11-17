//! Struct and variant pattern lowering through references.

use crate::operand::Operand;
use crate::place::Projection;
use crate::rvalue::Rvalue;

use super::support::{assignment, function, lower_clean};

#[test]
fn a_variant_pattern_reads_the_tag_through_a_reference() {
    let (fixture, hir, mir) = lower_clean(
        "enum Mode { On, Off(i32) } \
         func f(mode: *Mode) -> i32 { match mode { Mode::Off(value) => value, Mode::On => 0 } }",
    );
    let body = function(&fixture, &hir, &mir, "f");
    let mut deref_tag = false;
    for block in body.blocks.iter() {
        for statement in &block.statements {
            let (_, rvalue) = assignment(statement);
            let Rvalue::Discriminant(place) = rvalue else {
                continue;
            };
            if place
                .projections
                .iter()
                .any(|projection| matches!(projection, Projection::Deref))
            {
                deref_tag = true;
            }
        }
    }
    assert!(
        deref_tag,
        "the tag is read after dereferencing the scrutinee"
    );
}

#[test]
fn a_struct_field_pattern_reads_through_a_reference() {
    let (fixture, hir, mir) = lower_clean(
        "struct Point { x: i32 } \
         func f(point: *Point) -> i32 { match point { Point { x } => x } }",
    );
    let body = function(&fixture, &hir, &mir, "f");
    let mut deref_field = false;
    for block in body.blocks.iter() {
        for statement in &block.statements {
            let (_, rvalue) = assignment(statement);
            let Rvalue::Use(Operand::Copy(place)) = rvalue else {
                continue;
            };
            let mut derefs = false;
            let mut fields = false;
            for projection in &place.projections {
                match projection {
                    Projection::Deref => derefs = true,
                    Projection::Field(_) => fields = true,
                    _ => {}
                }
            }
            if derefs && fields {
                deref_field = true;
            }
        }
    }
    assert!(
        deref_field,
        "the binding reads a field of the dereferenced scrutinee"
    );
}
