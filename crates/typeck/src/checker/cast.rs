//! Explicit cast checking and diagnostics.
//!
//! `as` converts between primitive numeric types only. The classification is
//! owned by [`seal_ty::TyCtxt::numeric_cast_kind`]; this module validates the
//! pair, reports one precise diagnostic when it is invalid, and records the
//! resulting kind for MIR and code generation.

use seal_error_codes as codes;
use seal_hir::HirExprId;
use seal_span::Span;
use seal_ty::{TyId, TyKind};

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

        let Some(kind) = self.hir.ty_ctxt().numeric_cast_kind(source, target) else {
            self.report_invalid_cast(source, target, operand, as_span, target_span);
            return self.error_ty();
        };
        self.results.record_cast(expr, kind);
        target
    }

    fn report_invalid_cast(
        &self,
        source: TyId,
        target: TyId,
        operand: HirExprId,
        as_span: Span,
        target_span: Span,
    ) {
        self.error(
            codes::E3048,
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
        .help(
            "`as` only converts between integer and floating-point types; \
             use `bool.to_i32()` or `char.to_u32()` for those named conversions",
        )
        .emit();
    }
}
