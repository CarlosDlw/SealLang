//! Substitution of type arguments into structural types.
//!
//! Substitution is deliberately shallow: it rewrites the finite [`TyId`] graph
//! it is given and never expands an aggregate's declaration. This keeps it
//! cheap and total even for recursive types; only instantiated fields are
//! substituted when a query asks for them.

use seal_hash::FxHashSet;

use crate::ctxt::TyCtxt;
use crate::id::{ParamTyId, SubstsId, TyId};
use crate::kind::TyKind;

/// Replaces every declared parameter in `ty` using `substs`.
pub(crate) fn substitute(ctxt: &TyCtxt, ty: TyId, substs: SubstsId) -> TyId {
    let args = ctxt.substs_args(substs);
    substitute_args(ctxt, ty, &args)
}

fn substitute_args(ctxt: &TyCtxt, ty: TyId, args: &[TyId]) -> TyId {
    match ctxt.kind(ty) {
        TyKind::Param(param) => {
            let data = ctxt.param_data(param);
            let index = data.index as usize;
            args.get(index).copied().unwrap_or_else(|| {
                panic!(
                    "type parameter `{}` has index {index}, but the substitution has {} args",
                    data.name.index(),
                    args.len()
                )
            })
        }
        TyKind::Ref { kind, pointee } => {
            let pointee = substitute_args(ctxt, pointee, args);
            ctxt.ref_ty_with(kind, pointee)
        }
        TyKind::Array { element, length } => {
            let element = substitute_args(ctxt, element, args);
            ctxt.array_ty(element, length)
        }
        TyKind::Slice { element } => {
            let element = substitute_args(ctxt, element, args);
            ctxt.slice_ty(element)
        }
        TyKind::Tuple(tuple) => {
            let elements: Vec<TyId> = ctxt
                .tuple_elements(tuple)
                .into_iter()
                .map(|element| substitute_args(ctxt, element, args))
                .collect();
            ctxt.tuple_ty(&elements)
        }
        TyKind::Adt { adt, args: inner } => {
            let inner_args = ctxt.substs_args(inner);
            let substituted: Vec<TyId> = inner_args
                .iter()
                .map(|argument| substitute_args(ctxt, *argument, args))
                .collect();
            let inner = if substituted == inner_args {
                inner
            } else {
                ctxt.substs(&substituted)
            };
            ctxt.adt_ty_with(adt, inner)
        }
        TyKind::Dyn {
            trait_def,
            args: inner,
        } => {
            let inner_args = ctxt.substs_args(inner);
            let substituted: Vec<TyId> = inner_args
                .iter()
                .map(|argument| substitute_args(ctxt, *argument, args))
                .collect();
            if substituted == inner_args {
                return ty;
            }
            ctxt.dyn_ty(trait_def, ctxt.substs(&substituted))
        }
        _ => ty,
    }
}

/// Returns the declared parameters that occur in `ty` in first-occurrence order.
pub(crate) fn free_params(ctxt: &TyCtxt, ty: TyId) -> Vec<ParamTyId> {
    let mut seen = FxHashSet::default();
    let mut params = Vec::new();
    collect_free_params(ctxt, ty, &mut seen, &mut params);
    params
}

fn collect_free_params(
    ctxt: &TyCtxt,
    ty: TyId,
    seen: &mut FxHashSet<ParamTyId>,
    params: &mut Vec<ParamTyId>,
) {
    match ctxt.kind(ty) {
        TyKind::Param(param) => {
            if seen.insert(param) {
                params.push(param);
            }
        }
        TyKind::Ref { pointee, .. } => collect_free_params(ctxt, pointee, seen, params),
        TyKind::Array { element, .. } | TyKind::Slice { element } => {
            collect_free_params(ctxt, element, seen, params);
        }
        TyKind::Tuple(tuple) => {
            for element in ctxt.tuple_elements(tuple) {
                collect_free_params(ctxt, element, seen, params);
            }
        }
        TyKind::Adt { args, .. } | TyKind::Dyn { args, .. } => {
            for argument in ctxt.substs_args(args) {
                collect_free_params(ctxt, argument, seen, params);
            }
        }
        _ => {}
    }
}

/// Returns `true` if `ty` mentions a declared parameter.
pub(crate) fn contains_param(ctxt: &TyCtxt, ty: TyId) -> bool {
    match ctxt.kind(ty) {
        TyKind::Param(_) => true,
        TyKind::Ref { pointee, .. } => contains_param(ctxt, pointee),
        TyKind::Array { element, .. } | TyKind::Slice { element } => contains_param(ctxt, element),
        TyKind::Tuple(tuple) => ctxt
            .tuple_elements(tuple)
            .into_iter()
            .any(|element| contains_param(ctxt, element)),
        TyKind::Adt { args, .. } | TyKind::Dyn { args, .. } => ctxt
            .substs_args(args)
            .into_iter()
            .any(|argument| contains_param(ctxt, argument)),
        _ => false,
    }
}
