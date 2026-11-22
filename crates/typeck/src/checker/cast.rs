//! Explicit cast checking and diagnostics.
//!
//! `as` converts between numeric types, references and raw pointers, and raw
//! pointers and pointer-sized integers. The classification is owned by
//! [`seal_ty::TyCtxt::cast_kind`]; this module validates the pair, proves the
//! trait implementation for an unsizing cast, reports one precise diagnostic
//! when it is invalid, and records the resulting kind for MIR and code
//! generation.

use seal_error_codes as codes;
use seal_hir::{ExprKind, HirExprId, Res};
use seal_span::Span;
use seal_ty::{CastKind, FnSigId, Instance, RawPtrKind, TraitRef, TyId, TyKind};

use super::trait_solver::Proof;

use super::Checker;
use super::flow::Flow;

impl Checker<'_> {
    /// Checks `operand as ty`, returning the target type on success.
    ///
    /// The operand is inferred without the target as an expected type, so
    /// `1 as u8` first types `1` as `i32` and then casts it, instead of
    /// treating the literal as a `u8` that must fit.
    pub(super) fn check_cast(
        &mut self,
        expr: HirExprId,
        operand: HirExprId,
        ty: TyId,
        as_span: Span,
        target_span: Span,
        flow: &mut Flow,
    ) -> TyId {
        let target = self.resolve_deep(ty);
        if let Some(definition) = self.hir.ty_ctxt().fn_ptr_parts(target) {
            return self.check_function_pointer_cast(
                operand,
                target,
                &definition,
                (as_span, target_span),
                flow,
            );
        }
        if let Some((pointer_kind, signature)) = self.function_pointer_target(target) {
            if pointer_kind.is_mut() {
                self.error(
                    codes::E3089,
                    target_span,
                    "a C function pointer cannot be `mut`",
                )
                .help("remove `mut`; function pointers are read-only")
                .emit();
                return self.error_ty();
            }
            if let ExprKind::Path { res, .. } = self.hir.expr(operand).kind.clone() {
                match res {
                    Res::Function(function) => {
                        return self.check_function_address_cast(
                            expr,
                            operand,
                            (function, signature),
                            target,
                            (as_span, target_span),
                        );
                    }
                    Res::TraitFunction { .. } => {
                        self.error(
                            codes::E3089,
                            as_span.cover(target_span),
                            "a trait method cannot become a C function pointer",
                        )
                        .help("convert a concrete function with no receiver instead")
                        .emit();
                        return self.error_ty();
                    }
                    _ => {}
                }
            }
        }

        let source = self.infer_expr(operand, None, flow);
        let source = self.resolve_deep(source);
        if self.is_error(source) || self.is_error(target) {
            return self.error_ty();
        }

        if let TyKind::Param(param) = self.hir.ty_ctxt().kind(source) {
            let name = self
                .interner
                .resolve(self.hir.ty_ctxt().param_data(param).name);
            self.error(
                codes::E3049,
                self.hir.expr(operand).span,
                format!(
                    "cannot cast type parameter `{name}` to `{}`",
                    self.ty_text(target)
                ),
            )
            .secondary(
                as_span.cover(target_span),
                "the cast must name a concrete numeric conversion",
            )
            .help(
                "Seal has no numeric-bound trait, so a cast on a type parameter \
                 cannot select an integer or floating-point conversion",
            )
            .emit();
            return self.error_ty();
        }

        if let TyKind::Param(param) = self.hir.ty_ctxt().kind(target) {
            let name = self
                .interner
                .resolve(self.hir.ty_ctxt().param_data(param).name);
            self.error(
                codes::E3049,
                target_span,
                format!(
                    "cannot cast `{}` to type parameter `{name}`",
                    self.ty_text(source)
                ),
            )
            .secondary(
                as_span.cover(target_span),
                "the target type is generic, not a numeric type",
            )
            .help(
                "Seal has no numeric-bound trait, so a cast to a type parameter \
                 cannot select an integer or floating-point conversion",
            )
            .emit();
            return self.error_ty();
        }

        let Some(kind) = self.hir.ty_ctxt().cast_kind(source, target) else {
            self.report_invalid_cast(source, target, operand, as_span, target_span);
            return self.error_ty();
        };
        if matches!(kind, CastKind::RefToRaw { .. })
            && self.unsizes_to_raw_trait_object(source, target)
            && !self.check_raw_trait_object(source, target, as_span.cover(target_span))
        {
            return self.error_ty();
        }
        if self.function_pointer_target(target).is_some() {
            self.require_unsafe_function_pointer(as_span.cover(target_span));
        }
        self.results.record_cast(expr, kind);
        target
    }

    /// Checks `operand as func(...)`.
    ///
    /// The operand must already be a Seal function pointer: a function item, a
    /// binding, or another pointer type. The conversion follows the same rule
    /// as an implicit coercion — identical parameters and result — and only the
    /// safety qualifier may strengthen from `func` to `unsafe func`. The
    /// address itself is unchanged, so MIR lowers the cast as a copy.
    fn check_function_pointer_cast(
        &mut self,
        operand: HirExprId,
        target: TyId,
        definition: &seal_ty::FnPtrDef,
        (as_span, target_span): (Span, Span),
        flow: &mut Flow,
    ) -> TyId {
        let source = self.infer_expr(operand, None, flow);
        let source = self.resolve_deep(source);
        if self.is_error(source) {
            return self.error_ty();
        }
        let Some(source_definition) = self.hir.ty_ctxt().fn_ptr_parts(source) else {
            self.error(
                codes::E3098,
                as_span.cover(target_span),
                format!(
                    "cannot cast `{}` to the function pointer type `{}`",
                    self.ty_text(source),
                    self.ty_text(target)
                ),
            )
            .help(
                "only a function item or another function pointer becomes a Seal function pointer",
            )
            .emit();
            return self.error_ty();
        };
        if source_definition.matches(definition)
            || (source_definition.params == definition.params
                && source_definition.ret == definition.ret
                && !source_definition.is_unsafe
                && definition.is_unsafe)
        {
            return target;
        }
        let (message, help) = if source_definition.is_unsafe && !definition.is_unsafe {
            (
                "cannot drop `unsafe` from a function pointer".to_owned(),
                "keep the pointer as `unsafe func(...)`, or call it inside an `unsafe` block",
            )
        } else {
            (
                format!(
                    "function pointer signatures differ: expected `{}`, found `{}`",
                    self.ty_text(target),
                    self.ty_text(source)
                ),
                "make the parameter and result types identical",
            )
        };
        self.error(codes::E3098, as_span.cover(target_span), message)
            .secondary(
                self.hir.expr(operand).span,
                format!("operand has type `{}`", self.ty_text(source)),
            )
            .help(help)
            .emit();
        self.error_ty()
    }

    /// Returns the pointer kind and signature when `ty` is a C function pointer.
    fn function_pointer_target(&self, ty: TyId) -> Option<(RawPtrKind, FnSigId)> {
        let tcx = self.hir.ty_ctxt();
        let (kind, pointee) = tcx.raw_parts(ty)?;
        Some((kind, tcx.fn_sig_id(pointee)?))
    }

    /// Checks a cast of a function item to a raw C function pointer.
    ///
    /// The source is not inferred as an ordinary value: a function item has no
    /// value type. Its declared signature is instantiated first, then compared
    /// field by field with the target signature. The instantiation is recorded
    /// for code generation, which emits a C-ABI trampoline around the Seal
    /// function so aggregate arguments and results follow the target's ABI.
    fn check_function_address_cast(
        &mut self,
        expr: HirExprId,
        operand: HirExprId,
        (function, signature): (seal_def::DefId, FnSigId),
        target: TyId,
        (as_span, target_span): (Span, Span),
    ) -> TyId {
        self.require_unsafe_function_pointer(as_span.cover(target_span));
        if self.hir.receiver(function).is_some() {
            self.error(
                codes::E3089,
                self.hir.expr(operand).span,
                "a method with a receiver cannot become a C function pointer",
            )
            .help("use a function that takes its state through a raw pointer parameter")
            .emit();
            return self.error_ty();
        }

        let explicit = self
            .hir
            .expr_args(operand)
            .map(|substs| self.hir.ty_ctxt().substs_args(substs));
        let explicit_function = self
            .hir
            .expr_function_args(operand)
            .map(|substs| self.hir.ty_ctxt().substs_args(substs));
        let all_generics = self.hir.flattened_generics(function).len();
        let own_generics = self
            .hir
            .generics(function)
            .map_or(0, |generics| generics.params.len());
        let has_explicit = explicit_function
            .as_ref()
            .or(explicit.as_ref())
            .is_some_and(|args| !args.is_empty());
        if all_generics > 0 && (own_generics == 0 || !has_explicit) {
            self.error(
                codes::E3089,
                self.hir.expr(operand).span,
                "the address of a generic function needs explicit type arguments",
            )
            .help("write the instantiation before the cast, as in `function::<i32> as ...`")
            .emit();
            return self.error_ty();
        }

        let from_impl = self.impl_of_function(function).is_some();
        let seed = if from_impl {
            match self.associated_seed(function, explicit.as_deref(), None, as_span) {
                Some(seed) => seed,
                None => return self.error_ty(),
            }
        } else {
            Default::default()
        };
        let explicit_own = explicit_function
            .as_deref()
            .or_else(|| (!from_impl).then_some(explicit.as_deref()).flatten());
        let instantiated = self.instantiate_function(function, &seed, explicit_own, None);
        let Some((params, ret, substs)) =
            self.finish_function_instantiation(expr, function, instantiated, as_span)
        else {
            return self.error_ty();
        };
        let declared = self.hir.ty_ctxt().fn_sig(signature);
        let variadic = self
            .hir
            .extern_function(function)
            .is_some_and(|declaration| declaration.variadic);
        if declared.params != params || declared.ret != ret || declared.variadic != variadic {
            let declared_text = self.ty_text(target);
            self.error(
                codes::E3089,
                as_span.cover(target_span),
                format!(
                    "the function signature does not match the C function pointer type `{declared_text}`"
                ),
            )
            .secondary(
                self.hir.expr(operand).span,
                format!(
                    "the function takes {} parameter(s) and returns `{}`",
                    params.len(),
                    self.ty_text(ret)
                ),
            )
            .help("make the parameter and return types identical, including variadicity")
            .emit();
            return self.error_ty();
        }

        self.results
            .record_function_address(expr, Instance::new(function, substs));
        target
    }

    /// Returns `true` when the cast turns a thin reference into a raw trait
    /// object, which needs the trait implementation and vtable proof.
    fn unsizes_to_raw_trait_object(&self, source: TyId, target: TyId) -> bool {
        let tcx = self.hir.ty_ctxt();
        let (
            TyKind::Ref { pointee, .. },
            TyKind::RawPtr {
                pointee: target_pointee,
                ..
            },
        ) = (tcx.kind(source), tcx.kind(target))
        else {
            return false;
        };
        !matches!(tcx.kind(pointee), TyKind::Dyn { .. })
            && matches!(tcx.kind(target_pointee), TyKind::Dyn { .. })
    }

    /// Proves that the source pointee implements the target trait object's
    /// principal trait.
    fn check_raw_trait_object(&mut self, source: TyId, target: TyId, span: Span) -> bool {
        let tcx = self.hir.ty_ctxt();
        let Some((_, source_pointee)) = tcx.ref_parts(source) else {
            return false;
        };
        let Some((_, target_pointee)) = tcx.raw_parts(target) else {
            return false;
        };
        let TyKind::Dyn { trait_def, args } = tcx.kind(target_pointee) else {
            return false;
        };
        if !self.is_sized(source_pointee) {
            self.error(
                codes::E3074,
                span,
                format!(
                    "cannot cast `{}` to the trait-object pointer `{}`",
                    self.ty_text(source),
                    self.ty_text(target)
                ),
            )
            .help("a trait object erases a sized concrete value")
            .emit();
            return false;
        }
        let goal = TraitRef::new(trait_def, source_pointee, args);
        match self.prove_trait(goal) {
            Proof::Proven => true,
            Proof::Missing => {
                self.error(
                    codes::E3046,
                    span,
                    format!(
                        "`{}` does not implement `{}`, so it cannot become a trait object",
                        self.ty_text(source_pointee),
                        self.ty_text(target_pointee)
                    ),
                )
                .help("implement the trait for the concrete type, or use a type that already does")
                .emit();
                false
            }
            Proof::Ambiguous => {
                self.error(
                    codes::E3046,
                    span,
                    format!(
                        "more than one implementation could make `{}` a `{}`",
                        self.ty_text(source_pointee),
                        self.ty_text(target_pointee)
                    ),
                )
                .help("remove the overlapping implementation, or disambiguate the trait arguments")
                .emit();
                false
            }
            Proof::Recursive => {
                self.error(
                    codes::E3046,
                    span,
                    format!(
                        "cannot prove a unique implementation of `{}` for `{}`",
                        self.ty_text(target_pointee),
                        self.ty_text(source_pointee)
                    ),
                )
                .help("the proof depends on itself; remove the recursive bound")
                .emit();
                false
            }
        }
    }

    fn report_invalid_cast(
        &self,
        source: TyId,
        target: TyId,
        operand: HirExprId,
        as_span: Span,
        target_span: Span,
    ) {
        let tcx = self.hir.ty_ctxt();
        let involves_raw = matches!(tcx.kind(source), TyKind::RawPtr { .. })
            || matches!(tcx.kind(target), TyKind::RawPtr { .. });
        let (code, help) = if involves_raw {
            (
                codes::E3074,
                "a reference becomes a raw pointer of the same pointee, a raw pointer \
                 changes to another raw pointer with compatible metadata, and only \
                 `usize`/`isize` convert to and from a thin raw pointer",
            )
        } else {
            (
                codes::E3048,
                "`as` only converts between integer and floating-point types; \
                 use `bool.to_i32()` or `char.to_u32()` for those named conversions",
            )
        };
        self.error(
            code,
            as_span.cover(target_span),
            format!(
                "cannot cast `{}` to `{}`",
                self.ty_text(source),
                self.ty_text(target)
            ),
        )
        .secondary(
            self.hir.expr(operand).span,
            format!("operand has type `{}`", self.ty_text(source)),
        )
        .secondary(
            target_span,
            format!("target type is `{}`", self.ty_text(target)),
        )
        .help(help)
        .emit();
    }
}
