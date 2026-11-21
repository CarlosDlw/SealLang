//! Slice ranges, slice references, and the built-in `len` operator.

use seal_ast::Literal;
use seal_error_codes as codes;
use seal_hir::{ExprKind, HirExprId};
use seal_span::Span;
use seal_ty::{RefKind, TyId, TyKind};

use super::flow::Flow;
use super::{Checker, PlaceMode};

/// The source shape a range is taken over.
struct SliceSource {
    element: TyId,
    length: Option<u64>,
    reference: Option<RefKind>,
}

impl Checker<'_> {
    /// Diagnoses a range that is not immediately borrowed.
    pub(super) fn check_slice(
        &mut self,
        base: HirExprId,
        start: Option<HirExprId>,
        end: Option<HirExprId>,
        span: Span,
        flow: &mut Flow,
    ) -> TyId {
        let base_ty = self.infer_expr_mode(base, None, flow, PlaceMode::Borrow);
        if let Some(start) = start {
            self.check_expr(start, self.usize_ty(), flow);
        }
        if let Some(end) = end {
            self.check_expr(end, self.usize_ty(), flow);
        }
        if !self.is_error(base_ty) {
            self.error(
                codes::E3024,
                span,
                "a slice range is unsized and must be borrowed",
            )
            .help("write `&values[start..end]` or `&mut values[start..end]`")
            .emit();
        }
        self.error_ty()
    }

    /// Types `&values[start..end]` as `*[T]`, checking bounds and mutability.
    pub(super) fn check_slice_ref(
        &mut self,
        base: HirExprId,
        start: Option<HirExprId>,
        end: Option<HirExprId>,
        kind: RefKind,
        span: Span,
        flow: &mut Flow,
    ) -> TyId {
        let base_ty = self.infer_expr_mode(base, None, flow, PlaceMode::Borrow);
        if self.is_error(base_ty) {
            self.check_bounds(start, end, self.usize_ty(), flow);
            return self.error_ty();
        }
        let Some(source) = self.slice_source(base_ty, base) else {
            if !self.is_error(base_ty) {
                self.error(
                    codes::E3024,
                    span,
                    format!("cannot slice a value of type `{}`", self.ty_text(base_ty)),
                )
                .help("only arrays and slice references can be sliced")
                .emit();
            }
            self.check_bounds(start, end, self.usize_ty(), flow);
            return self.error_ty();
        };

        self.check_bounds(start, end, self.usize_ty(), flow);
        if let Some(length) = source.length {
            self.check_constant_bounds(start, end, length);
        }

        if kind.is_exclusive() {
            match source.reference {
                Some(reference) if reference.is_exclusive() => {}
                Some(_) => {
                    self.error(
                        codes::E3017,
                        span,
                        "cannot mutably slice through a shared reference",
                    )
                    .help("`*T` is read-only; use a `*mut T` reference to write through it")
                    .emit();
                }
                None if !self.borrow_is_writable(base) => {
                    self.report_mutable_borrow(base);
                }
                None => {}
            }
        }

        let slice = self.slice_ty(source.element);
        self.ref_ty(kind, slice)
    }

    /// Returns the slice type of `len(values)`.
    pub(super) fn check_len(&mut self, operand: HirExprId, span: Span, flow: &mut Flow) -> TyId {
        let ty = self.infer_expr(operand, None, flow);
        if self.is_error(ty) {
            return self.error_ty();
        }
        let is_slice = match self.hir.ty_ctxt().kind(ty) {
            TyKind::Ref { pointee, .. } => self.slice_parts(pointee).is_some(),
            _ => false,
        };
        if !is_slice {
            self.error(
                codes::E3025,
                span,
                format!(
                    "`len` requires a slice reference, found `{}`",
                    self.ty_text(ty)
                ),
            )
            .help("pass a `*[T]` or `*mut [T]`, such as `len(&values[..])`")
            .emit();
            return self.error_ty();
        }
        self.usize_ty()
    }

    /// Checks the built-in `assert` intrinsic.
    ///
    /// The message was validated during lowering, so only the condition needs
    /// a type: it must be `bool`, and the expression itself produces `()`.
    pub(super) fn check_assert(
        &mut self,
        condition: HirExprId,
        span: Span,
        flow: &mut Flow,
    ) -> TyId {
        let bool_ty = self.bool_ty();
        let condition_ty = self.infer_expr(condition, Some(bool_ty), flow);
        if condition_ty != bool_ty && !self.is_error(condition_ty) {
            self.mismatch(span, bool_ty, condition_ty);
        }
        self.unit()
    }

    fn slice_source(&self, ty: TyId, base: HirExprId) -> Option<SliceSource> {
        match self.hir.ty_ctxt().kind(ty) {
            TyKind::Array { element, length } => Some(SliceSource {
                element,
                length: Some(length),
                reference: None,
            }),
            TyKind::Ref { kind, pointee } => match self.hir.ty_ctxt().kind(pointee) {
                TyKind::Array { element, length } => Some(SliceSource {
                    element,
                    length: Some(length),
                    reference: Some(kind),
                }),
                TyKind::Slice { element } => Some(SliceSource {
                    element,
                    length: None,
                    reference: Some(kind),
                }),
                _ => None,
            },
            TyKind::Error => {
                let _ = base;
                Some(SliceSource {
                    element: ty,
                    length: None,
                    reference: None,
                })
            }
            _ => None,
        }
    }

    fn check_bounds(
        &mut self,
        start: Option<HirExprId>,
        end: Option<HirExprId>,
        expected: TyId,
        flow: &mut Flow,
    ) {
        for bound in [start, end].into_iter().flatten() {
            self.check_expr(bound, expected, flow);
        }
    }

    fn check_constant_bounds(&self, start: Option<HirExprId>, end: Option<HirExprId>, length: u64) {
        let start_value = start.and_then(|bound| self.usize_literal(bound));
        let end_value = end.and_then(|bound| self.usize_literal(bound));
        if let Some(bound) = start
            && let Some(value) = start_value
            && value > u128::from(length)
        {
            self.report_bound_out_of_range(bound, value, length, "start");
        }
        if let Some(bound) = end
            && let Some(value) = end_value
            && value > u128::from(length)
        {
            self.report_bound_out_of_range(bound, value, length, "end");
        }
        if let (Some(start), Some(start_value), Some(end_value)) = (start, start_value, end_value)
            && start_value > end_value
            && start_value <= u128::from(length)
            && end_value <= u128::from(length)
        {
            self.error(
                codes::E3026,
                self.hir.expr(start).span,
                "the slice start is greater than its end",
            )
            .help("write a range whose start is at most its end")
            .emit();
        }
    }

    fn report_bound_out_of_range(&self, bound: HirExprId, value: u128, length: u64, name: &str) {
        self.error(
            codes::E3022,
            self.hir.expr(bound).span,
            format!("slice {name} `{value}` is out of bounds for a sequence of length {length}"),
        )
        .help("a start may be at most `length`; an end may be at most `length`")
        .emit();
    }

    fn usize_literal(&self, expr: HirExprId) -> Option<u128> {
        let ExprKind::Lit(Literal::Int {
            value, ref suffix, ..
        }) = self.hir.expr(expr).kind
        else {
            return None;
        };
        suffix
            .is_none_or(|suffix| self.interner.resolve(suffix) == "usize")
            .then_some(value)
    }
}
