//! Enum variant construction in MIR.

use seal_ty::VariantIdx;

use crate::rvalue::{AggregateKind, Rvalue};

use super::support::{assignment, function, lower_clean};

#[test]
fn a_payload_variant_lowers_to_a_variant_aggregate() {
    let (fixture, hir, mir) = lower_clean(
        "enum Shape { Empty, Circle { radius: i32 } } \
         func main() -> Shape { Shape::Circle { radius: 1 } }",
    );
    let body = function(&fixture, &hir, &mir, "main");
    let block = body.block(body.entry);
    let (_, rvalue) = assignment(&block.statements[0]);
    let Rvalue::Aggregate { kind, fields } = rvalue else {
        panic!("expected an aggregate, got {rvalue:?}");
    };
    assert_eq!(
        *kind,
        AggregateKind::Variant {
            adt: kind_adt(*kind),
            variant: VariantIdx::from_u32(1).expect("small index"),
        }
    );
    assert_eq!(fields.len(), 1);
    assert_eq!(
        fixture.symbol(hir.definitions().name(fields[0].0)),
        "radius"
    );
}

#[test]
fn a_unit_variant_lowers_to_an_empty_variant_aggregate() {
    let (fixture, hir, mir) = lower_clean(
        "enum Shape { Empty, Circle { radius: i32 } } \
         func main() -> Shape { Shape::Empty }",
    );
    let body = function(&fixture, &hir, &mir, "main");
    let (_, rvalue) = assignment(&body.block(body.entry).statements[0]);
    let Rvalue::Aggregate { kind, fields } = rvalue else {
        panic!("expected an aggregate, got {rvalue:?}");
    };
    let AggregateKind::Variant { variant, .. } = kind else {
        panic!("expected a variant aggregate, got {kind:?}");
    };
    assert_eq!(*variant, VariantIdx::from_u32(0).expect("small index"));
    assert!(fields.is_empty());
}

fn kind_adt(kind: AggregateKind) -> seal_def::DefId {
    match kind {
        AggregateKind::Variant { adt, .. } | AggregateKind::Struct(adt) => adt,
    }
}
