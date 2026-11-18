//! Implicit coercions applied at expected-type boundaries.
//!
//! The only coercion in the language is pointer unsizing:
//!
//! - `*S` becomes `*dyn Trait` when `S` implements the trait;
//! - `*mut S` becomes `*mut dyn Trait` under the same condition.
//!
//! The source and target reference kinds must match. Coercions are recorded on
//! the expression so MIR and monomorphization consume the decision instead of
//! repeating proof search.

use seal_error_codes as codes;
use seal_hir::HirExprId;
use seal_ty::{TraitRef, TyKind};

use super::Checker;
use super::flow::Flow;
use super::trait_solver::Proof;
use crate::results::Coercion;

impl Checker<'_> {
    /// Checks `expr` against `expected`, applying a trait-object coercion when
    /// the natural source type differs only by pointer unsizing.
    ///
    /// Returns the expression's effective type: the expected type after a
    /// successful coercion, or the natural type when no coercion applies.
    pub(super) fn check_expr_against(
        &mut self,
        expr: HirExprId,
        expected: seal_ty::TyId,
        flow: &mut Flow,
    ) -> seal_ty::TyId {
        let expected = self.resolve_deep(expected);
        let found = self.infer_expr(expr, Some(expected), flow);
        if self.is_compatible(expected, found) {
            return found;
        }

        match self.coerce_to_trait_object(expr, found, expected) {
            Ok(true) => {
                self.results.record_coercion(
                    expr,
                    Coercion::TraitObject {
                        source: found,
                        target: expected,
                    },
                );
                self.results.record_expr(expr, expected);
                expected
            }
            Ok(false) => {
                let span = self.hir.expr(expr).span;
                self.mismatch(span, expected, found);
                found
            }
            Err(()) => {
                self.results.record_expr(expr, self.error_ty());
                self.error_ty()
            }
        }
    }

    /// Attempts to coerce `source` to `target`.
    ///
    /// `Ok(false)` means the target is not a trait-object pointer, so ordinary
    /// type equality applies. `Err(())` means the target is a trait-object
    /// pointer and the conversion was rejected with a diagnostic.
    fn coerce_to_trait_object(
        &mut self,
        expr: HirExprId,
        source: seal_ty::TyId,
        target: seal_ty::TyId,
    ) -> Result<bool, ()> {
        let tcx = self.hir.ty_ctxt();
        let Some((target_kind, target_pointee)) = tcx.ref_parts(target) else {
            return Ok(false);
        };
        let TyKind::Dyn { trait_def, args } = tcx.kind(target_pointee) else {
            return Ok(false);
        };

        let span = self.hir.expr(expr).span;
        let Some((source_kind, source_pointee)) = tcx.ref_parts(source) else {
            self.error(
                codes::E3047,
                span,
                format!(
                    "cannot coerce `{}` to the trait-object pointer `{}`",
                    self.ty_text(source),
                    self.ty_text(target)
                ),
            )
            .help("a trait-object coercion requires a pointer to a concrete value")
            .emit();
            return Err(());
        };

        if matches!(tcx.kind(source_pointee), TyKind::Dyn { .. }) {
            self.error(
                codes::E3047,
                span,
                "cannot coerce one trait-object pointer to another",
            )
            .help("trait-object upcasting is not supported; use the principal trait's object")
            .emit();
            return Err(());
        }
        if source_kind != target_kind {
            self.error(
                codes::E3047,
                span,
                format!(
                    "cannot coerce `{}` to `{}` because the reference kinds differ",
                    self.ty_text(source),
                    self.ty_text(target)
                ),
            )
            .help("reborrow the pointer explicitly, as in `&*pointer`, to share a `*mut` value")
            .emit();
            return Err(());
        }
        if !self.is_sized(source_pointee) {
            self.error(
                codes::E3047,
                span,
                format!(
                    "cannot coerce `{}` because its pointee is unsized",
                    self.ty_text(source)
                ),
            )
            .help("only a pointer to a sized concrete value can become a trait object")
            .emit();
            return Err(());
        }

        let goal = TraitRef::new(trait_def, source_pointee, args);
        match self.prove_trait(goal) {
            Proof::Proven => Ok(true),
            Proof::Missing => {
                self.report_missing_trait_object_impl(span, source_pointee, target_pointee);
                Err(())
            }
            Proof::Ambiguous => {
                self.report_ambiguous_trait_object_impl(span, source_pointee, target_pointee);
                Err(())
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
                Err(())
            }
        }
    }

    fn report_missing_trait_object_impl(
        &self,
        span: seal_span::Span,
        source: seal_ty::TyId,
        target: seal_ty::TyId,
    ) {
        self.error(
            codes::E3046,
            span,
            format!(
                "`{}` does not implement `{}`, so it cannot become a trait object",
                self.ty_text(source),
                self.ty_text(target)
            ),
        )
        .help("implement the trait for the concrete type, or use a type that already does")
        .emit();
    }

    fn report_ambiguous_trait_object_impl(
        &self,
        span: seal_span::Span,
        source: seal_ty::TyId,
        target: seal_ty::TyId,
    ) {
        self.error(
            codes::E3046,
            span,
            format!(
                "more than one implementation could make `{}` a `{}`",
                self.ty_text(source),
                self.ty_text(target)
            ),
        )
        .help("remove the overlapping implementation, or disambiguate the trait arguments")
        .emit();
    }
}
