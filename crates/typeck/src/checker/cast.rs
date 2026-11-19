//! Explicit cast checking and diagnostics.
//!
//! `as` converts between numeric types, references and raw pointers, and raw
//! pointers and pointer-sized integers. The classification is owned by
//! [`seal_ty::TyCtxt::cast_kind`]; this module validates the pair, proves the
//! trait implementation for an unsizing cast, reports one precise diagnostic
//! when it is invalid, and records the resulting kind for MIR and code
//! generation.

use seal_error_codes as codes;
use seal_hir::HirExprId;
use seal_span::Span;
use seal_ty::{CastKind, TraitRef, TyId, TyKind};

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
        let source = self.infer_expr(operand, None, flow);
        let source = self.resolve_deep(source);
        let target = self.resolve_deep(ty);
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
        self.results.record_cast(expr, kind);
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
