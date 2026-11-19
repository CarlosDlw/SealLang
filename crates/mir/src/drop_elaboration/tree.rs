//! Expanding a drop site into the values that a destructor actually visits.
//!
//! One source-level drop can cover a value whose fields are dropped in a
//! specific order. A struct without a destructor drops its fields one by one, in
//! declaration order, while an enum, an array, or a type with a declared
//! destructor is dropped as one unit: its glue reads the active variant, walks
//! the elements, or runs the destructor first. Drop elaboration records that
//! distinction as a list of places, so the generated code calls glue exactly
//! once per value and never field by field where the language says otherwise.

use seal_hir::Hir;
use seal_ty::{AdtKind, TyCtxt, TyId, TyKind};

use crate::body::Body;
use crate::place::Place;
use crate::validate::place_ty;

/// The deepest field nesting one site is expanded through.
///
/// Drop expansion mirrors the type structure, so a malformed or cyclic type
/// would recurse without bound. The guard turns that into one drop of the whole
/// value, which is what glue would do anyway; a by-value cycle is already a type
/// error before elaboration runs.
const MAX_DROP_DEPTH: usize = 32;

/// Returns the places one drop of `place` must visit, in drop order.
///
/// A type that needs no destructor yields nothing. A type with a declared
/// destructor, an enum, and an array each yield the place itself, because their
/// glue handles the inside of the value. A struct without a destructor yields
/// its fields, recursively, so the caller can skip the fields that were already
/// moved out.
pub(crate) fn drop_nodes(hir: &Hir, body: &Body, place: &Place) -> Vec<Place> {
    let mut nodes = Vec::new();
    let ty = place_ty(hir, body, place);
    collect(hir.ty_ctxt(), ty, place.clone(), &mut nodes, 0);
    nodes
}

fn collect(tcx: &TyCtxt, ty: TyId, place: Place, nodes: &mut Vec<Place>, depth: usize) {
    if !tcx.needs_drop(ty) {
        return;
    }
    if depth >= MAX_DROP_DEPTH {
        nodes.push(place);
        return;
    }
    match tcx.kind(ty) {
        TyKind::Adt { adt, args } => {
            let definition = tcx.adt(adt);
            // A declared destructor answers for the whole aggregate, and an
            // enum's glue selects the active variant, so neither is expanded.
            if definition.kind == AdtKind::Enum || tcx.drop_method(adt).is_some() {
                nodes.push(place);
                return;
            }
            for field in &definition.fields {
                let field_place = place.clone().field(field.def);
                let field_ty = tcx.field_ty(adt, args, field.def).unwrap_or(field.ty);
                collect(tcx, field_ty, field_place, nodes, depth + 1);
            }
        }
        TyKind::Tuple(tuple) => {
            for (index, element) in tcx.tuple_elements(tuple).iter().enumerate() {
                let element_place = place.clone().tuple_field(index as u32);
                collect(tcx, *element, element_place, nodes, depth + 1);
            }
        }
        // A declared parameter, an array, and anything else that reaches here
        // are dropped by glue as a whole. An array's glue iterates its
        // elements, and a parameter is replaced by its concrete type during
        // monomorphization, which produces the right glue then.
        _ => nodes.push(place),
    }
}
