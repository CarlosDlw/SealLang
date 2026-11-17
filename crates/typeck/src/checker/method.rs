//! Intrinsic method resolution.
//!
//! Lookup is deliberately explicit: the receiver's reference layers are peeled
//! until a scalar, array, or slice reference is found, then the catalog chooses
//! one entry. The resolved call records every dereference and the way the
//! receiver reaches MIR, so later phases never repeat inference.

use seal_error_codes as codes;
use seal_hir::HirExprId;
use seal_methods::{MethodDef, MethodId, MethodOwner, ParamKind, ReceiverMode, ReturnKind};
use seal_span::Span;
use seal_symbol::Symbol;
use seal_ty::{RefKind, TyId, TyKind};

use super::Checker;
use super::flow::Flow;
use crate::results::{ReceiverForm, ResolvedMethod};

const MAX_RECEIVER_DEREFS: usize = 16;

/// The checked pieces of one `receiver.name(arguments)` expression.
pub(super) struct MethodCall<'a> {
    pub(super) expr: HirExprId,
    pub(super) receiver: HirExprId,
    pub(super) name: Symbol,
    pub(super) name_span: Span,
    pub(super) args: &'a [HirExprId],
    pub(super) span: Span,
}

impl Checker<'_> {
    pub(super) fn check_method_call(&mut self, call: MethodCall<'_>, flow: &mut Flow) -> TyId {
        let MethodCall {
            expr,
            receiver,
            name,
            name_span,
            args,
            span,
        } = call;
        let receiver_ty = self.infer_expr(receiver, None, flow);
        let receiver_place = self.results.place(receiver).cloned();
        let resolution = self.resolve_method(
            receiver,
            receiver_ty,
            receiver_place.as_ref(),
            name,
            name_span,
            receiver_ty,
        );
        let Some(resolution) = resolution else {
            for arg in args {
                self.infer_expr(*arg, None, flow);
            }
            return self.error_ty();
        };

        if resolution.def.receiver == ReceiverMode::Exclusive
            && !self.receiver_is_writable(receiver, receiver_place.as_ref(), &resolution)
        {
            self.error(
                codes::E3029,
                self.hir.expr(receiver).span,
                format!(
                    "method `{}` needs to modify its receiver",
                    resolution.def.name
                ),
            )
            .help("borrow the receiver exclusively or declare the binding with `let mut`")
            .emit();
        }

        if resolution.def.requires_copy_element && !self.is_copy(resolution.element_ty) {
            self.error(
                codes::E3030,
                span,
                format!(
                    "method `fill` needs a `Copy` element, but `{}` is not `Copy`",
                    self.ty_text(resolution.element_ty)
                ),
            )
            .help("copy the value into each element explicitly, or use a `Copy` element type")
            .emit();
        }

        if resolution.def.params.len() != args.len() {
            self.error(
                codes::E3028,
                span,
                format!(
                    "method `{}` takes {} argument(s) but {} were supplied",
                    resolution.def.name,
                    resolution.def.params.len(),
                    args.len()
                ),
            )
            .help("pass exactly one argument for each parameter")
            .emit();
        }

        let mut arg_tys = Vec::with_capacity(args.len());
        for (index, arg) in args.iter().enumerate() {
            match resolution.def.params.get(index) {
                Some(kind) => {
                    let expected = self.param_ty(*kind, &resolution);
                    self.check_expr(*arg, expected, flow);
                    arg_tys.push(expected);
                }
                None => {
                    self.infer_expr(*arg, None, flow);
                }
            }
        }

        let ret_ty = self.return_ty(&resolution);
        if !self.is_error(ret_ty) {
            self.results.record_method(
                expr,
                ResolvedMethod {
                    method: resolution.id,
                    receiver_ty: resolution.receiver_ty,
                    receiver_form: resolution.form,
                    receiver_derefs: resolution.derefs,
                    array_to_slice: resolution.array_to_slice,
                    arg_tys,
                    ret_ty,
                },
            );
        }
        ret_ty
    }

    fn resolve_method(
        &self,
        receiver: HirExprId,
        receiver_ty: TyId,
        receiver_place: Option<&crate::results::ResolvedPlace>,
        name: Symbol,
        name_span: Span,
        original_ty: TyId,
    ) -> Option<Resolution> {
        let mut ty = receiver_ty;
        let mut derefs = 0;
        let mut refs = Vec::new();
        let mut slice_ref_kind = None;
        let (owner, base_ty, form) = loop {
            match self.hir.ty_ctxt().kind(ty) {
                TyKind::Ref { kind, pointee } => {
                    if let TyKind::Slice { element: _ } = self.hir.ty_ctxt().kind(pointee) {
                        slice_ref_kind = Some(kind);
                        break (MethodOwner::Slice, pointee, ReceiverForm::Value);
                    }
                    refs.push(kind);
                    derefs += 1;
                    if derefs > MAX_RECEIVER_DEREFS {
                        self.error(
                            codes::E3027,
                            name_span,
                            "method receiver has too many reference layers",
                        )
                        .help("dereference the receiver explicitly before calling the method")
                        .emit();
                        return None;
                    }
                    ty = pointee;
                }
                TyKind::Int(_) => break (MethodOwner::Int, ty, ReceiverForm::Place),
                TyKind::Uint(_) => break (MethodOwner::Uint, ty, ReceiverForm::Place),
                TyKind::Float(_) => break (MethodOwner::Float, ty, ReceiverForm::Place),
                TyKind::Bool => break (MethodOwner::Bool, ty, ReceiverForm::Place),
                TyKind::Char => break (MethodOwner::Char, ty, ReceiverForm::Place),
                TyKind::Array { .. } => break (MethodOwner::Array, ty, ReceiverForm::Place),
                TyKind::Error => return None,
                _ => {
                    self.report_missing_method(original_ty, name, name_span);
                    return None;
                }
            }
        };
        let _ = receiver;

        let method_name = self.interner.resolve(name);
        let mut array_to_slice = false;
        let id = match seal_methods::find(owner, method_name) {
            Some(id) => id,
            None if owner == MethodOwner::Array => {
                match seal_methods::find(MethodOwner::Slice, method_name) {
                    Some(id) => {
                        array_to_slice = true;
                        id
                    }
                    None => {
                        self.report_missing_method(original_ty, name, name_span);
                        return None;
                    }
                }
            }
            None => {
                self.report_missing_method(original_ty, name, name_span);
                return None;
            }
        };
        let def = id.def();
        let element_ty = match self.hir.ty_ctxt().kind(base_ty) {
            TyKind::Array { element, .. } => element,
            TyKind::Slice { element } => element,
            _ => base_ty,
        };
        if def.receiver == ReceiverMode::Exclusive && slice_ref_kind == Some(RefKind::Shared) {
            self.error(
                codes::E3029,
                self.hir.expr(receiver).span,
                format!("method `{}` cannot write through a shared slice", def.name),
            )
            .help("use a `*mut [T]` receiver, or declare the slice binding as mutable")
            .emit();
        }
        if def.receiver == ReceiverMode::Exclusive && derefs > 0 && refs.contains(&RefKind::Shared)
        {
            self.error(
                codes::E3029,
                self.hir.expr(receiver).span,
                format!(
                    "method `{}` cannot write through a shared reference",
                    def.name
                ),
            )
            .help("dereference a `*mut T` receiver, or declare the binding with `let mut`")
            .emit();
        }
        if def.receiver == ReceiverMode::Exclusive && derefs == 0 && form == ReceiverForm::Place {
            if let Some(place) = receiver_place {
                let definition = self.hir.definitions().get(place.local);
                let writable = definition.mutable || place.deref == Some(RefKind::Exclusive);
                if !writable {
                    self.error(
                        codes::E3029,
                        self.hir.expr(receiver).span,
                        format!(
                            "method `{}` modifies `{}`, which is not a mutable place",
                            def.name,
                            self.ty_text(base_ty)
                        ),
                    )
                    .help("declare the binding with `let mut`, or use a `*mut T` receiver")
                    .emit();
                }
            }
        }

        Some(Resolution {
            id,
            def,
            receiver_ty: base_ty,
            form,
            derefs,
            array_to_slice,
            element_ty,
        })
    }

    fn receiver_is_writable(
        &self,
        receiver: HirExprId,
        place: Option<&crate::results::ResolvedPlace>,
        resolution: &Resolution,
    ) -> bool {
        if resolution.form == ReceiverForm::Value {
            return true;
        }
        if resolution.derefs > 0 {
            return true;
        }
        match place {
            Some(place) => {
                self.hir.definitions().get(place.local).mutable
                    || place.deref == Some(RefKind::Exclusive)
            }
            None => {
                // A temporary has its own storage, which exists only for this
                // call. It can be modified even though no effect is observable.
                let _ = receiver;
                true
            }
        }
    }

    fn report_missing_method(&self, receiver_ty: TyId, name: Symbol, span: Span) {
        let name = self.interner.resolve(name);
        self.error(
            codes::E3027,
            span,
            format!(
                "no method named `{name}` is available for `{}`",
                self.ty_text(receiver_ty)
            ),
        )
        .help("check the method name, or dereference the receiver to the type that defines it")
        .emit();
    }

    fn param_ty(&self, kind: ParamKind, resolution: &Resolution) -> TyId {
        match kind {
            ParamKind::SelfValue => resolution.element_ty,
            ParamKind::Element => resolution.element_ty,
            ParamKind::Usize => self.usize_ty(),
            ParamKind::I32 => self.i32_ty(),
            ParamKind::U32 => self.u32_ty(),
        }
    }

    fn return_ty(&self, resolution: &Resolution) -> TyId {
        match resolution.def.ret {
            ReturnKind::Same => resolution.element_ty,
            ReturnKind::Bool => self.bool_ty(),
            ReturnKind::Usize => self.usize_ty(),
            ReturnKind::I32 => self.i32_ty(),
            ReturnKind::U32 => self.u32_ty(),
            ReturnKind::Unit => self.unit(),
        }
    }
}

struct Resolution {
    id: MethodId,
    def: &'static MethodDef,
    receiver_ty: TyId,
    form: ReceiverForm,
    derefs: usize,
    array_to_slice: bool,
    /// The scalar type for scalar owners, or the element type for sequences.
    element_ty: TyId,
}
