//! Intrinsic method resolution.
//!
//! Lookup is deliberately explicit: the receiver's reference layers are peeled
//! until a scalar, array, or slice reference is found, then the catalog chooses
//! one entry. The resolved call records every dereference and the way the
//! receiver reaches MIR, so later phases never repeat inference.

use seal_def::DefId;
use seal_error_codes as codes;
use seal_hir::{HirExprId, ReceiverKind, TraitMethodLookup};
use seal_methods::{MethodDef, MethodId, MethodOwner, ParamKind, ReceiverMode, ReturnKind};
use seal_span::Span;
use seal_symbol::Symbol;
use seal_ty::{RefKind, TyId, TyKind};

use super::Checker;
use super::flow::Flow;
use crate::results::{MethodKind, ReceiverAdjust, ReceiverForm, ResolvedMethod, ResolvedPlace};

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

        let target = match self.user_method_target(receiver_ty, name, name_span) {
            UserLookup::NotAStruct => None,
            UserLookup::Rejected => {
                for arg in args {
                    self.infer_expr(*arg, None, flow);
                }
                return self.error_ty();
            }
            UserLookup::Method(target) => Some(target),
        };
        if let Some(target) = target {
            return self.check_user_method(
                UserCall {
                    expr,
                    receiver,
                    receiver_ty,
                    receiver_place,
                    target,
                    name,
                    args,
                    span,
                },
                flow,
            );
        }

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
                    kind: MethodKind::Intrinsic(resolution.id),
                    receiver_adjust: ReceiverAdjust::None,
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

    /// Looks up a user method on a nominal receiver before intrinsic lookup.
    ///
    /// An inherent function wins; otherwise the traits the type implements are
    /// searched. The receiver is not a struct in [`UserLookup::NotAStruct`], so
    /// the caller falls through to the intrinsic catalog. Every struct receiver
    /// that fails stops here: the error is reported once and the call does not
    /// also produce a catalog diagnostic.
    fn user_method_target(&self, receiver_ty: TyId, name: Symbol, name_span: Span) -> UserLookup {
        let mut base = receiver_ty;
        while let TyKind::Ref { pointee, .. } = self.hir.ty_ctxt().kind(base) {
            base = pointee;
        }
        let TyKind::Adt(adt) = self.hir.ty_ctxt().kind(base) else {
            return UserLookup::NotAStruct;
        };
        let type_def = self.hir.ty_ctxt().adt(adt).def;

        if let Some(function) = self.hir.method(type_def, name) {
            return match self.hir.receiver(function) {
                Some(receiver) => UserLookup::Method(UserMethod {
                    type_def,
                    function,
                    receiver,
                    source: MethodSource::Inherent,
                }),
                None => {
                    self.report_associated_as_method(base, name, name_span);
                    UserLookup::Rejected
                }
            };
        }

        match self.hir.trait_method(type_def, name) {
            TraitMethodLookup::Missing => {
                self.report_missing_method(receiver_ty, name, name_span);
                UserLookup::Rejected
            }
            TraitMethodLookup::Ambiguous(traits) => {
                self.report_ambiguous_trait_method(base, name, name_span, &traits);
                UserLookup::Rejected
            }
            TraitMethodLookup::Found {
                trait_def,
                function,
            } => match self.hir.receiver(function) {
                Some(receiver) => UserLookup::Method(UserMethod {
                    type_def,
                    function,
                    receiver,
                    source: MethodSource::Trait(trait_def),
                }),
                None => {
                    self.report_associated_as_method(base, name, name_span);
                    UserLookup::Rejected
                }
            },
        }
    }

    fn report_associated_as_method(&self, base: TyId, name: Symbol, span: Span) {
        self.error(
            codes::E2019,
            span,
            format!(
                "`{}` is an associated function, not a method",
                self.interner.resolve(name)
            ),
        )
        .help(format!(
            "call it as `{}::{}()`, or give it a `self` receiver",
            self.ty_text(base),
            self.interner.resolve(name)
        ))
        .emit();
    }

    fn report_ambiguous_trait_method(
        &self,
        base: TyId,
        name: Symbol,
        span: Span,
        traits: &[DefId],
    ) {
        let traits = traits
            .iter()
            .map(|definition| {
                format!(
                    "`{}`",
                    self.interner
                        .resolve(self.hir.definitions().name(*definition))
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        self.error(
            codes::E2028,
            span,
            format!(
                "the method `{}` on `{}` is declared by more than one trait",
                self.interner.resolve(name),
                self.ty_text(base)
            ),
        )
        .help(format!(
            "the traits are {traits}; rename the method in one of them, or add an inherent method"
        ))
        .emit();
    }

    fn check_user_method(&mut self, call: UserCall<'_>, flow: &mut Flow) -> TyId {
        let UserCall {
            expr,
            receiver,
            receiver_ty,
            receiver_place,
            target,
            name,
            args,
            span,
        } = call;
        let type_def = target.type_def;
        let function = target.function;
        let receiver_kind = target.receiver;

        let signature = self
            .results
            .signature(function)
            .expect("every impl function has a signature")
            .clone();
        let adt = self
            .hir
            .adt(type_def)
            .expect("every struct has a reserved aggregate");
        let base_ty = self.hir.ty_ctxt().adt_ty(adt);
        let expected = match receiver_kind {
            ReceiverKind::Value => base_ty,
            ReceiverKind::Shared => self.ref_ty(RefKind::Shared, base_ty),
            ReceiverKind::Exclusive => self.ref_ty(RefKind::Exclusive, base_ty),
        };

        let adjust = if receiver_ty == expected {
            ReceiverAdjust::None
        } else if receiver_ty == base_ty {
            match receiver_kind {
                ReceiverKind::Value => ReceiverAdjust::None,
                ReceiverKind::Shared => ReceiverAdjust::SharedBorrow,
                ReceiverKind::Exclusive => ReceiverAdjust::ExclusiveBorrow,
            }
        } else if receiver_kind == ReceiverKind::Shared
            && matches!(
                self.hir.ty_ctxt().kind(receiver_ty),
                TyKind::Ref {
                    kind: RefKind::Exclusive,
                    pointee,
                } if pointee == base_ty
            )
        {
            ReceiverAdjust::SharedReborrow
        } else {
            self.error(
                codes::E3031,
                self.hir.expr(receiver).span,
                format!(
                    "method `{}` expects `{}`, but the receiver has type `{}`",
                    self.interner.resolve(name),
                    self.ty_text(expected),
                    self.ty_text(receiver_ty)
                ),
            )
            .help(
                "adjust the receiver to match the method signature; a `*mut T` receiver can be reborrowed as `*T` for a read-only method",
            )
            .emit();
            ReceiverAdjust::None
        };

        if adjust == ReceiverAdjust::ExclusiveBorrow
            && !self.receiver_place_is_writable(receiver_place.as_ref())
        {
            self.error(
                codes::E3029,
                self.hir.expr(receiver).span,
                format!(
                    "method `{}` needs to modify its receiver",
                    self.interner.resolve(name)
                ),
            )
            .help("declare the binding with `let mut`, or pass an exclusive reference")
            .emit();
        }

        let expected_args = signature.params.len().saturating_sub(1);
        if expected_args != args.len() {
            self.error(
                codes::E3028,
                span,
                format!(
                    "method `{}` takes {} argument(s) but {} were supplied",
                    self.interner.resolve(name),
                    expected_args,
                    args.len()
                ),
            )
            .help("pass exactly one argument for each parameter after the receiver")
            .emit();
        }

        let mut arg_tys = Vec::with_capacity(args.len());
        for (index, arg) in args.iter().enumerate() {
            match signature.params.get(index + 1) {
                Some(expected) => {
                    self.check_expr(*arg, *expected, flow);
                    arg_tys.push(*expected);
                }
                None => {
                    self.infer_expr(*arg, None, flow);
                }
            }
        }

        let ret_ty = signature.ret;
        if !self.is_error(ret_ty) {
            let kind = match target.source {
                MethodSource::Inherent => MethodKind::User {
                    function,
                    receiver: receiver_kind,
                },
                MethodSource::Trait(trait_def) => MethodKind::Trait {
                    function,
                    receiver: receiver_kind,
                    trait_def,
                },
            };
            self.results.record_method(
                expr,
                ResolvedMethod {
                    kind,
                    receiver_adjust: adjust,
                    receiver_ty: base_ty,
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

    fn receiver_place_is_writable(&self, place: Option<&ResolvedPlace>) -> bool {
        match place {
            Some(place) => {
                self.hir.definitions().get(place.local).mutable
                    || place.deref == Some(RefKind::Exclusive)
            }
            None => true,
        }
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

/// The user-defined resolution chosen for a method call.
struct UserMethod {
    type_def: DefId,
    function: DefId,
    receiver: ReceiverKind,
    source: MethodSource,
}

/// Where a user method was declared.
#[derive(Clone, Copy)]
enum MethodSource {
    /// An inherent `impl Type` block.
    Inherent,
    /// An `impl Trait for Type` block, with the trait definition.
    Trait(DefId),
}

/// The result of looking for a user method before the intrinsic catalog.
enum UserLookup {
    /// The receiver is not a struct, so intrinsic lookup applies.
    NotAStruct,
    /// The call resolved to a user method.
    Method(UserMethod),
    /// The receiver is a struct and the error was already reported.
    Rejected,
}

/// The checked pieces of a user method call.
struct UserCall<'a> {
    expr: HirExprId,
    receiver: HirExprId,
    receiver_ty: TyId,
    receiver_place: Option<ResolvedPlace>,
    target: UserMethod,
    name: Symbol,
    args: &'a [HirExprId],
    span: Span,
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
