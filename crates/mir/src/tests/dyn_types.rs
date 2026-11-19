//! Trait-object coercions, reborrows, and dynamic calls in MIR.

use seal_layout::VtableLayout;
use seal_ty::TyKind;

use crate::body::Body;
use crate::rvalue::Rvalue;
use crate::statement::StatementKind;
use crate::validate::ValidationError;

use super::support::{function, lower_clean};

const SHAPE: &str = "\
struct Circle { radius: i32 } \
trait Shape { func area(self: *Self) -> i32; } \
impl Shape for Circle { func area(self: *Circle) -> i32 { (*self).radius } }";

const SCALE: &str = "\
struct Meter { value: i32 } \
trait Scale { func scale(self: *Self, factor: i32) -> i32; } \
impl Scale for Meter { func scale(self: *Meter, factor: i32) -> i32 { (*self).value * factor } }";

fn first_rvalue(body: &Body, predicate: impl Fn(&Rvalue) -> bool) -> &Rvalue {
    body.blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .map(|statement| {
            let StatementKind::Assign { rvalue, .. } = &statement.kind else {
                panic!("expected an assignment, found {:?}", statement.kind);
            };
            rvalue
        })
        .find(|rvalue| predicate(rvalue))
        .expect("the body contains the expected rvalue")
}

#[test]
fn coercion_lowers_to_make_trait_object() {
    let source = format!("{SHAPE} func upcast(circle: *Circle) -> *dyn Shape {{ circle }}");
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "upcast");
    let Rvalue::MakeTraitObject {
        source_ty,
        target_ty,
        ..
    } = first_rvalue(body, |rvalue| {
        matches!(rvalue, Rvalue::MakeTraitObject { .. })
    })
    else {
        unreachable!();
    };
    assert_eq!(fixture.ty_text(&hir, *source_ty), "*Circle");
    assert_eq!(fixture.ty_text(&hir, *target_ty), "*dyn Shape");
}

#[test]
fn dynamic_call_lowers_to_a_vtable_slot() {
    let source = format!("{SHAPE} func area(object: *dyn Shape) -> i32 {{ object.area() }}");
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "area");
    let Rvalue::DynamicCall {
        object_trait_ref,
        method,
        slot,
        ..
    } = first_rvalue(body, |rvalue| matches!(rvalue, Rvalue::DynamicCall { .. }))
    else {
        unreachable!();
    };
    let layout = VtableLayout::new(hir.ty_ctxt(), *object_trait_ref)
        .expect("the principal trait has a layout");
    assert_eq!(layout.slot_of(*method), Some(*slot));
}

#[test]
fn dynamic_call_uses_the_trait_method_signature() {
    let source = format!("{SCALE} func apply(object: *dyn Scale) -> i32 {{ object.scale(3) }}");
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "apply");
    let Rvalue::DynamicCall {
        arg_tys, ret_ty, ..
    } = first_rvalue(body, |rvalue| matches!(rvalue, Rvalue::DynamicCall { .. }))
    else {
        unreachable!();
    };
    assert_eq!(arg_tys.len(), 1);
    assert_eq!(fixture.ty_text(&hir, arg_tys[0]), "i32");
    assert_eq!(fixture.ty_text(&hir, *ret_ty), "i32");
}

#[test]
fn reborrow_preserves_data_and_vtable() {
    let source =
        format!("{SHAPE} func shared(object: *mut dyn Shape) -> *dyn Shape {{ &*object }}");
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "shared");
    let Rvalue::TraitObjectReborrow { from, to, .. } = first_rvalue(body, |rvalue| {
        matches!(rvalue, Rvalue::TraitObjectReborrow { .. })
    }) else {
        unreachable!();
    };
    assert_eq!(fixture.ty_text(&hir, *from), "*mut dyn Shape");
    assert_eq!(fixture.ty_text(&hir, *to), "*dyn Shape");
    let (_, from_object) = hir.ty_ctxt().ref_parts(*from).expect("from is a pointer");
    let (_, to_object) = hir.ty_ctxt().ref_parts(*to).expect("to is a pointer");
    assert_eq!(from_object, to_object);
}

#[test]
fn reborrow_does_not_create_a_new_vtable() {
    let source =
        format!("{SHAPE} func shared(object: *mut dyn Shape) -> *dyn Shape {{ &*object }}");
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "shared");
    let rvalues: Vec<&Rvalue> = body
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .map(|statement| {
            let StatementKind::Assign { rvalue, .. } = &statement.kind else {
                panic!("expected an assignment, found {:?}", statement.kind);
            };
            rvalue
        })
        .collect();
    assert_eq!(
        rvalues
            .iter()
            .filter(|rvalue| matches!(rvalue, Rvalue::TraitObjectReborrow { .. }))
            .count(),
        1
    );
    assert!(
        !rvalues
            .iter()
            .any(|rvalue| matches!(rvalue, Rvalue::MakeTraitObject { .. }))
    );
}

#[test]
fn mir_validation_rejects_a_thin_dyn_pointer() {
    let source = format!("{SHAPE} func upcast(circle: *Circle) -> *dyn Shape {{ circle }}");
    let (fixture, hir, mir) = lower_clean(&source);
    let mut body = function(&fixture, &hir, &mir, "upcast").clone();
    let statement = body
        .blocks
        .iter_mut()
        .flat_map(|block| block.statements.iter_mut())
        .find(|statement| {
            matches!(
                statement.kind,
                StatementKind::Assign {
                    rvalue: Rvalue::MakeTraitObject { .. },
                    ..
                }
            )
        })
        .expect("upcast builds a trait object");
    let StatementKind::Assign {
        rvalue:
            Rvalue::MakeTraitObject {
                source_ty,
                target_ty,
                ..
            },
        ..
    } = &mut statement.kind
    else {
        unreachable!();
    };
    *source_ty = *target_ty;
    assert_eq!(
        body.validate_with(&hir),
        Err(ValidationError::TraitObjectSourceNotThin { block: body.entry })
    );
}

#[test]
fn mir_validation_rejects_a_sharedness_change() {
    let source = format!("{SHAPE} func upcast(circle: *Circle) -> *dyn Shape {{ circle }}");
    let (fixture, hir, mir) = lower_clean(&source);
    let mut body = function(&fixture, &hir, &mir, "upcast").clone();
    let statement = body
        .blocks
        .iter_mut()
        .flat_map(|block| block.statements.iter_mut())
        .find(|statement| {
            matches!(
                statement.kind,
                StatementKind::Assign {
                    rvalue: Rvalue::MakeTraitObject { .. },
                    ..
                }
            )
        })
        .expect("upcast builds a trait object");
    let StatementKind::Assign {
        rvalue: Rvalue::MakeTraitObject { target_ty, .. },
        ..
    } = &mut statement.kind
    else {
        unreachable!();
    };
    let pointee = hir
        .ty_ctxt()
        .ref_parts(*target_ty)
        .expect("the target is a pointer")
        .1;
    *target_ty = hir.ty_ctxt().mut_ref_ty(pointee);
    assert_eq!(
        body.validate_with(&hir),
        Err(ValidationError::TraitObjectKindMismatch { block: body.entry })
    );
}

#[test]
fn mir_validation_rejects_a_bad_vtable_slot() {
    let source = format!("{SHAPE} func area(object: *dyn Shape) -> i32 {{ object.area() }}");
    let (fixture, hir, mir) = lower_clean(&source);
    let mut body = function(&fixture, &hir, &mir, "area").clone();
    let statement = body
        .blocks
        .iter_mut()
        .flat_map(|block| block.statements.iter_mut())
        .find(|statement| {
            matches!(
                statement.kind,
                StatementKind::Assign {
                    rvalue: Rvalue::DynamicCall { .. },
                    ..
                }
            )
        })
        .expect("area calls dynamically");
    let StatementKind::Assign {
        rvalue: Rvalue::DynamicCall { slot, .. },
        ..
    } = &mut statement.kind
    else {
        unreachable!();
    };
    *slot = seal_layout::VtableSlot::new(u32::MAX);
    assert_eq!(
        body.validate_with(&hir),
        Err(ValidationError::DynamicSlotMismatch { block: body.entry })
    );
}

#[test]
fn dynamic_call_keeps_generic_arguments_until_mono() {
    let source = format!("{SHAPE} func upcast<T: Shape>(value: *T) -> *dyn Shape {{ value }}");
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "upcast");
    let Rvalue::MakeTraitObject { source_ty, .. } = first_rvalue(body, |rvalue| {
        matches!(rvalue, Rvalue::MakeTraitObject { .. })
    }) else {
        unreachable!();
    };
    let (_, pointee) = hir
        .ty_ctxt()
        .ref_parts(*source_ty)
        .expect("the source is a pointer");
    assert!(matches!(hir.ty_ctxt().kind(pointee), TyKind::Param(_)));
    body.validate_with(&hir)
        .expect("generic types are accepted before monomorphization");
}
