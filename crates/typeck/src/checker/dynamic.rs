//! Dynamic method lookup on trait-object receivers.
//!
//! The receiver's principal trait fixes the vtable. Methods declared by the
//! principal win; otherwise the flattened supertrait graph is searched. The
//! chosen method is recorded with its slot, and no concrete implementation
//! instance is recorded, because dispatch happens at run time.

use seal_error_codes as codes;
use seal_hir::{ExprKind, HirExprId, ReceiverKind};
use seal_layout::VtableLayout;
use seal_span::Span;
use seal_symbol::Symbol;
use seal_ty::{RefKind, TraitMethodDef, TraitRef, TyId, TyKind};

use super::Checker;
use super::flow::Flow;
use super::method::trait_receiver_kind;
use crate::results::{MethodKind, ReceiverAdjust, ReceiverForm, ResolvedMethod};

/// One method call whose receiver may be a trait object.
pub(super) struct DynamicCall<'a> {
    pub(super) expr: HirExprId,
    pub(super) receiver: HirExprId,
    pub(super) receiver_ty: TyId,
    pub(super) name: Symbol,
    pub(super) name_span: Span,
    pub(super) generic_args: Option<seal_ty::SubstsId>,
    pub(super) args: &'a [HirExprId],
    pub(super) span: Span,
    pub(super) expected: Option<TyId>,
}

/// How the receiver reaches its trait object.
#[derive(Clone, Copy)]
struct DynamicReceiver {
    /// The `dyn Trait` type the receiver points at or names.
    object: TyId,
    /// The principal trait reference used to select the vtable.
    object_trait_ref: TraitRef,
    /// How the source pointer permits access.
    source_kind: RefKind,
    /// Whether the receiver is a dyn place behind an explicit dereference,
    /// rather than a pointer value.
    from_place: bool,
}

impl Checker<'_> {
    /// Handles a method call on a trait object.
    ///
    /// Returns `None` when the receiver is not a trait object, so ordinary
    /// method lookup continues.
    pub(super) fn check_dynamic_method_call(
        &mut self,
        call: DynamicCall<'_>,
        flow: &mut Flow,
    ) -> Option<TyId> {
        let receiver = self.dynamic_receiver(call.receiver, call.receiver_ty)?;
        Some(self.check_dynamic_call(call, receiver, flow))
    }

    /// Recognizes `*dyn Trait`, `*mut dyn Trait`, and an explicit `*pointer`
    /// dyn place.
    fn dynamic_receiver(&self, receiver: HirExprId, receiver_ty: TyId) -> Option<DynamicReceiver> {
        let tcx = self.hir.ty_ctxt();
        if let Some((kind, pointee)) = tcx.ref_parts(receiver_ty)
            && matches!(tcx.kind(pointee), TyKind::Dyn { .. })
        {
            let (trait_def, args) = tcx.dyn_parts(pointee)?;
            return Some(DynamicReceiver {
                object: pointee,
                object_trait_ref: TraitRef::new(trait_def, pointee, args),
                source_kind: kind,
                from_place: false,
            });
        }

        if !matches!(tcx.kind(receiver_ty), TyKind::Dyn { .. }) {
            return None;
        }
        let ExprKind::Deref { operand: pointer } = self.hir.expr(receiver).kind else {
            return None;
        };
        let pointer_ty = self.results.expr_ty(pointer)?;
        let (kind, pointee) = tcx.ref_parts(pointer_ty)?;
        if pointee != receiver_ty {
            return None;
        }
        let (trait_def, args) = tcx.dyn_parts(receiver_ty)?;
        Some(DynamicReceiver {
            object: receiver_ty,
            object_trait_ref: TraitRef::new(trait_def, receiver_ty, args),
            source_kind: kind,
            from_place: true,
        })
    }

    fn check_dynamic_call(
        &mut self,
        call: DynamicCall<'_>,
        receiver: DynamicReceiver,
        flow: &mut Flow,
    ) -> TyId {
        let DynamicCall {
            expr,
            receiver: receiver_expr,
            name,
            name_span,
            generic_args,
            args,
            span,
            expected,
            ..
        } = call;

        let Some((declaring_trait, method)) = self.dynamic_method_target(receiver, name, name_span)
        else {
            self.infer_arguments(args, flow);
            return self.error_ty();
        };
        let Some(declared_receiver) = method.receiver else {
            self.report_associated_as_method(receiver.object, name, name_span);
            self.infer_arguments(args, flow);
            return self.error_ty();
        };
        let declared = trait_receiver_kind(declared_receiver);
        if declared == ReceiverKind::Value {
            // Object safety already rejected this declaration at the `dyn` use.
            self.infer_arguments(args, flow);
            return self.error_ty();
        }
        if !method.generics.params.is_empty() {
            // Generic methods are not dyn-compatible; HIR reported E2065 at the
            // `dyn` use, so do not add a cascading type error.
            self.infer_arguments(args, flow);
            return self.error_ty();
        }

        let Some(adjust) =
            self.dynamic_receiver_adjustment(receiver_expr, receiver, declared, name, name_span)
        else {
            self.infer_arguments(args, flow);
            return self.error_ty();
        };

        let own_count = method.generics.params.len();
        let explicit = self.method_type_arguments(generic_args, own_count, span);
        let Some(instantiated) =
            self.instantiate_trait_method(declaring_trait, &method, explicit.as_deref(), expected)
        else {
            self.infer_arguments(args, flow);
            return self.error_ty();
        };
        if instantiated.params.len() != args.len() {
            self.error(
                codes::E3028,
                span,
                format!(
                    "method `{}` takes {} argument(s) but {} were supplied",
                    self.interner.resolve(method.name),
                    instantiated.params.len(),
                    args.len()
                ),
            )
            .help("pass exactly one argument for each parameter after the receiver")
            .emit();
        }
        for (index, arg) in args.iter().enumerate() {
            match instantiated.params.get(index) {
                Some(expected) => {
                    self.check_expr_against(*arg, *expected, flow);
                }
                None => {
                    self.infer_expr(*arg, None, flow);
                }
            }
        }
        let Some(args_ty) = self.finish_arguments(&instantiated.generics, &instantiated.args, span)
        else {
            return self.error_ty();
        };
        let substs = self.hir.ty_ctxt().substs(&args_ty);
        let arg_tys: Vec<TyId> = method
            .signature
            .params
            .iter()
            .map(|parameter| self.substitute(*parameter, substs))
            .collect();
        let ret_ty = self.substitute(method.signature.ret, substs);

        let slot = match VtableLayout::new(self.hir.ty_ctxt(), receiver.object_trait_ref)
            .ok()
            .and_then(|layout| layout.slot_of(method.def))
        {
            Some(slot) => slot,
            None => {
                self.error(
                    codes::E3047,
                    span,
                    format!(
                        "method `{}` has no slot in the trait-object vtable",
                        self.interner.resolve(method.name)
                    ),
                )
                .help("this is an internal vtable inconsistency; report the program that triggered it")
                .emit();
                return self.error_ty();
            }
        };

        if !self.is_error(ret_ty) {
            self.results.record_method(
                expr,
                ResolvedMethod {
                    kind: MethodKind::Dynamic {
                        object_trait_ref: receiver.object_trait_ref,
                        method: method.def,
                        slot,
                        receiver: declared,
                    },
                    receiver_adjust: adjust,
                    receiver_ty: receiver.object,
                    receiver_form: ReceiverForm::Place,
                    receiver_derefs: 0,
                    array_to_slice: false,
                    arg_tys,
                    ret_ty,
                },
            );
        }
        ret_ty
    }

    /// Selects the dispatched method from the principal and its supertraits.
    fn dynamic_method_target(
        &mut self,
        receiver: DynamicReceiver,
        name: Symbol,
        name_span: Span,
    ) -> Option<(TraitRef, TraitMethodDef)> {
        let refs = self.hir.ty_ctxt().dyn_trait_refs(receiver.object);
        let mut candidates: Vec<(TraitRef, TraitMethodDef)> = Vec::new();
        for (position, trait_ref) in refs.iter().copied().enumerate() {
            let Some(definition) = self.hir.trait_definition(trait_ref.trait_def) else {
                continue;
            };
            let Some(method) = definition.method(name) else {
                continue;
            };
            if position == 0 {
                candidates.clear();
                candidates.push((trait_ref, method.clone()));
                break;
            }
            if !candidates
                .iter()
                .any(|(_, candidate)| candidate.def == method.def)
            {
                candidates.push((trait_ref, method.clone()));
            }
        }

        match candidates.len() {
            0 => {
                self.report_missing_method(receiver.object, name, name_span);
                None
            }
            1 => candidates.pop(),
            _ => {
                let traits: Vec<seal_def::DefId> = candidates
                    .iter()
                    .map(|(trait_ref, _)| trait_ref.trait_def)
                    .collect();
                self.report_ambiguous_trait_method(receiver.object, name, name_span, &traits);
                None
            }
        }
    }

    /// Chooses how the receiver reaches the vtable-dispatched method.
    fn dynamic_receiver_adjustment(
        &mut self,
        receiver_expr: HirExprId,
        receiver: DynamicReceiver,
        declared: ReceiverKind,
        name: Symbol,
        name_span: Span,
    ) -> Option<ReceiverAdjust> {
        let adjust = match (receiver.from_place, receiver.source_kind, declared) {
            (false, RefKind::Shared, ReceiverKind::Shared) => ReceiverAdjust::None,
            (false, RefKind::Exclusive, ReceiverKind::Exclusive) => ReceiverAdjust::None,
            (false, RefKind::Exclusive, ReceiverKind::Shared) => ReceiverAdjust::SharedReborrow,
            (false, RefKind::Shared, ReceiverKind::Exclusive) => {
                self.report_shared_exclusive_dispatch(receiver_expr, name, name_span);
                return None;
            }
            (true, _, ReceiverKind::Shared) => ReceiverAdjust::SharedBorrow,
            (true, RefKind::Exclusive, ReceiverKind::Exclusive) => ReceiverAdjust::ExclusiveBorrow,
            (true, RefKind::Shared, ReceiverKind::Exclusive) => {
                self.report_shared_exclusive_dispatch(receiver_expr, name, name_span);
                return None;
            }
            (_, _, ReceiverKind::Value) => return None,
        };
        Some(adjust)
    }

    fn report_shared_exclusive_dispatch(&self, receiver: HirExprId, name: Symbol, name_span: Span) {
        self.error(
            codes::E3029,
            name_span,
            format!(
                "method `{}` needs exclusive access through the trait object",
                self.interner.resolve(name)
            ),
        )
        .secondary(self.hir.expr(receiver).span, "shared trait object")
        .help("use a `*mut dyn Trait` receiver to call a method that takes `*mut Self`")
        .emit();
    }

    fn infer_arguments(&mut self, args: &[HirExprId], flow: &mut Flow) {
        for arg in args {
            self.infer_expr(*arg, None, flow);
        }
    }
}
