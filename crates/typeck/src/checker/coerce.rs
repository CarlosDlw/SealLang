//! Implicit coercions applied at expected-type boundaries.
//!
//! Two coercions exist:
//!
//! - `*S` becomes `*dyn Trait` when `S` implements the trait, and `*mut S`
//!   becomes `*mut dyn Trait` under the same condition;
//! - a safe `func(...)` becomes an `unsafe func(...)` with the same signature.
//!
//! The source and target reference kinds must match. A trait-object coercion is
//! recorded on the expression so MIR and monomorphization consume the decision
//! instead of repeating proof search. Function-pointer safety is a property of
//! the same address, so it needs no MIR operation.

use seal_error_codes as codes;
use seal_hir::HirExprId;
use seal_ty::{TraitRef, TyKind};

use super::Checker;
use super::PlaceMode;
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
        self.check_expr_in_mode(expr, expected, flow, PlaceMode::Read)
    }

    /// Checks `expr` against `expected` as a value that is read, not moved.
    ///
    /// An equality operand may be a non-`Copy` value reached through a
    /// reference or stored in a static; the comparison only reads it, so owning
    /// that storage is not required.
    pub(super) fn check_expr_read(
        &mut self,
        expr: HirExprId,
        expected: seal_ty::TyId,
        flow: &mut Flow,
    ) -> seal_ty::TyId {
        self.check_expr_in_mode(expr, expected, flow, PlaceMode::Borrow)
    }

    fn check_expr_in_mode(
        &mut self,
        expr: HirExprId,
        expected: seal_ty::TyId,
        flow: &mut Flow,
        mode: PlaceMode,
    ) -> seal_ty::TyId {
        let expected = self.resolve_deep(expected);
        let found = self.infer_expr_mode(expr, Some(expected), flow, mode);
        if self.is_compatible(expected, found) {
            return found;
        }

        match self.coerce_to_fn_trait(expr, found, expected) {
            Ok(true) => {
                // The value keeps its closure or pointer type; the bound only
                // constrains how the generic parameter may be used.
                self.results.record_expr(expr, found);
                found
            }
            Err(()) => {
                self.results.record_expr(expr, self.error_ty());
                self.error_ty()
            }
            Ok(false) => self.check_expr_after_fn_trait(expr, found, expected),
        }
    }

    fn check_expr_after_fn_trait(
        &mut self,
        expr: HirExprId,
        found: seal_ty::TyId,
        expected: seal_ty::TyId,
    ) -> seal_ty::TyId {
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
            Ok(false) => match self.coerce_to_closure_pointer(expr, found, expected) {
                Ok(true) => {
                    let closure = self
                        .closure_of_ty(found)
                        .expect("a closure-pointer coercion has a closure source");
                    self.results.record_coercion(
                        expr,
                        Coercion::ClosureToFnPtr {
                            closure,
                            target: expected,
                        },
                    );
                    self.results.record_expr(expr, expected);
                    expected
                }
                Ok(false) => match self.coerce_to_function_pointer(expr, found, expected) {
                    Ok(true) => {
                        // The address is the same value; only the signature it
                        // may be called with changed, so no MIR rvalue is needed.
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
                },
                Err(()) => {
                    self.results.record_expr(expr, self.error_ty());
                    self.error_ty()
                }
            },
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

    /// Attempts to coerce a closure without captures to a function pointer.
    ///
    /// `Ok(false)` means the source is not a closure, so another coercion or
    /// ordinary equality applies. `Err(())` means the source is a closure but
    /// cannot become the target pointer, and a diagnostic was reported.
    fn coerce_to_closure_pointer(
        &mut self,
        expr: HirExprId,
        source: seal_ty::TyId,
        target: seal_ty::TyId,
    ) -> Result<bool, ()> {
        let Some(closure) = self.closure_of_ty(source) else {
            return Ok(false);
        };
        let Some(target_def) = self.hir.ty_ctxt().fn_ptr_parts(target) else {
            return Ok(false);
        };
        let Some(info) = self.results.closure_info(closure).cloned() else {
            return Ok(false);
        };
        let span = self.hir.expr(expr).span;
        if !info.captures.is_empty() {
            self.error(
                codes::E3098,
                span,
                format!(
                    "this closure captures {} value(s), so it cannot become a function pointer",
                    info.captures.len()
                ),
            )
            .help(
                "use a closure without captures, or take the captured values as parameters instead",
            )
            .emit();
            return Err(());
        }

        let params: Vec<seal_ty::TyId> = info
            .signature
            .params
            .iter()
            .map(|ty| self.resolve_deep(*ty))
            .collect();
        let ret = self.resolve_deep(info.signature.ret);
        if params != target_def.params || ret != target_def.ret {
            // The closure body already reported the signature mismatch with
            // `E3094` when the same expected type supplied its signature.
            return Err(());
        }
        Ok(true)
    }

    /// Attempts to coerce one Seal function pointer to another.
    ///
    /// `Ok(false)` means the target is not a function pointer, so ordinary
    /// equality applies. `Err(())` means the conversion was rejected with
    /// `E3098`. Only the safety qualifier may change: the parameters and the
    /// result must be identical, and a safe pointer may become `unsafe`.
    fn coerce_to_function_pointer(
        &mut self,
        expr: HirExprId,
        source: seal_ty::TyId,
        target: seal_ty::TyId,
    ) -> Result<bool, ()> {
        let tcx = self.hir.ty_ctxt();
        let Some(target_def) = tcx.fn_ptr_parts(target) else {
            return Ok(false);
        };
        let Some(source_def) = tcx.fn_ptr_parts(source) else {
            return Ok(false);
        };
        if source_def.matches(&target_def)
            || (source_def.params == target_def.params
                && source_def.ret == target_def.ret
                && !source_def.is_unsafe
                && target_def.is_unsafe)
        {
            return Ok(true);
        }

        let span = self.hir.expr(expr).span;
        let (message, help) = if source_def.is_unsafe && !target_def.is_unsafe {
            (
                "cannot drop `unsafe` from a function pointer".to_owned(),
                "keep the pointer as `unsafe func(...)`, or call it inside an `unsafe` block",
            )
        } else if source_def.params != target_def.params || source_def.ret != target_def.ret {
            (
                format!(
                    "function pointer signatures differ: expected `{}`, found `{}`",
                    self.ty_text(target),
                    self.ty_text(source)
                ),
                "make the parameter and result types identical",
            )
        } else {
            (
                "this function cannot become the expected function pointer".to_owned(),
                "use a function item with the same signature and safety qualifier",
            )
        };
        self.error(codes::E3098, span, message).help(help).emit();
        Err(())
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
