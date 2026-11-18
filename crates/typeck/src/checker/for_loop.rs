//! Type checking for `for` loops.

use seal_ast::{Literal, UnaryOp};
use seal_error_codes as codes;
use seal_hir::{ExprKind, ForSource, HirBlockId, HirExprId, HirPatternId};
use seal_span::Span;
use seal_ty::{TyId, TyKind};

use super::Checker;
use super::flow::Flow;

impl Checker<'_> {
    /// Checks a `for` loop and returns the flow after evaluating its source.
    ///
    /// The iterable is evaluated once before the loop, so a move in the source
    /// stays moved after the loop. The body starts from a copy of that flow with
    /// the pattern bindings initialized, and its changes do not escape: the loop
    /// may run zero times.
    pub(super) fn check_for(
        &mut self,
        pattern: HirPatternId,
        source: ForSource,
        body: HirBlockId,
        span: Span,
        mut flow: Flow,
    ) -> Flow {
        let mut custom_iteration = None;
        let element_ty = match source {
            ForSource::Range { start, end, .. } => {
                self.check_for_range(start, end, span, &mut flow)
            }
            ForSource::Expr(iterable) => {
                let iterable_ty = self.infer_expr(iterable, None, &mut flow);
                let iterable_span = self.hir.expr(iterable).span;
                if self.is_builtin_iteration_source(iterable_ty) {
                    self.for_element_ty(iterable_ty, iterable_span)
                } else if self.is_error(iterable_ty) {
                    iterable_ty
                } else {
                    match self.resolve_iteration(iterable, iterable_ty, iterable_span) {
                        Some(protocol) => {
                            let item_ty = protocol.item_ty;
                            custom_iteration = Some((iterable, iterable_ty, protocol));
                            item_ty
                        }
                        None => self.error_ty(),
                    }
                }
            }
        };

        let mut body_flow = flow.clone();
        self.check_pattern(pattern, element_ty, &mut body_flow);
        if !self.for_pattern_is_irrefutable(pattern, element_ty) {
            let pattern_span = self.hir.pattern(pattern).span;
            self.error(
                codes::E3034,
                pattern_span,
                "this pattern does not match every element",
            )
            .help("bind every element with a name or `_`; a literal or variant test skips values")
            .emit();
        }
        let _body = self.check_block(body, None, body_flow);
        if let Some((iterable, iterable_ty, protocol)) = custom_iteration {
            self.finalize_iteration(iterable, pattern, iterable_ty, protocol, span);
        }
        flow
    }

    /// Returns whether a `for` source uses built-in sequence iteration.
    ///
    /// Ranges are handled before this classification. Arrays, slices, and one
    /// reference layer to either keep the existing loop lowering; any other
    /// type may resolve through `IntoIterator` or `Iterator`.
    fn is_builtin_iteration_source(&self, ty: TyId) -> bool {
        match self.hir.ty_ctxt().kind(ty) {
            TyKind::Array { .. } | TyKind::Slice { .. } => true,
            TyKind::Ref { pointee, .. } => matches!(
                self.hir.ty_ctxt().kind(pointee),
                TyKind::Array { .. } | TyKind::Slice { .. }
            ),
            _ => false,
        }
    }

    /// Checks the bounds of an integer range and returns their common type.
    ///
    /// An unsuffixed integer literal on one side takes the type of the other
    /// side, so `0..values.len()` uses the `usize` the method returns instead of
    /// defaulting the literal to `i32`.
    fn check_for_range(
        &mut self,
        start: HirExprId,
        end: HirExprId,
        span: Span,
        flow: &mut Flow,
    ) -> TyId {
        let start_is_literal = self.is_unsuffixed_int(start);
        let end_is_literal = self.is_unsuffixed_int(end);
        let ty = if start_is_literal && !end_is_literal {
            let end_ty = self.infer_expr(end, None, flow);
            self.check_expr(start, end_ty, flow);
            end_ty
        } else {
            let start_ty = self.infer_expr(start, None, flow);
            self.check_expr(end, start_ty, flow);
            start_ty
        };

        if !self.is_integer(ty) && !self.is_error(ty) {
            self.error(
                codes::E3035,
                span,
                format!(
                    "a for range needs integer bounds, found `{}`",
                    self.ty_text(ty)
                ),
            )
            .help("iterate an integer range, such as `0..values.len()`")
            .emit();
            return self.error_ty();
        }
        ty
    }

    /// Returns whether `expr` is an unsuffixed integer literal, possibly under
    /// a leading `-`.
    fn is_unsuffixed_int(&self, expr: HirExprId) -> bool {
        let mut current = expr;
        loop {
            match &self.hir.expr(current).kind {
                ExprKind::Lit(Literal::Int { suffix: None, .. }) => return true,
                ExprKind::Unary {
                    op: UnaryOp::Neg,
                    operand,
                } => current = *operand,
                _ => return false,
            }
        }
    }

    /// Returns the type bound by a `for` pattern for this iterable.
    fn for_element_ty(&mut self, ty: TyId, span: Span) -> TyId {
        match self.hir.ty_ctxt().kind(ty) {
            TyKind::Array { element, .. } => element,
            TyKind::Ref { kind, pointee } => match self.hir.ty_ctxt().kind(pointee) {
                TyKind::Array { element, .. } | TyKind::Slice { element } => {
                    self.hir.ty_ctxt().ref_ty_with(kind, element)
                }
                _ => {
                    self.error(
                        codes::E3035,
                        span,
                        format!("cannot iterate over `{}`", self.ty_text(ty)),
                    )
                    .help("iterate an array, `&array`, or a slice reference such as `*[T]`")
                    .emit();
                    self.error_ty()
                }
            },
            TyKind::Slice { .. } => {
                self.error(
                    codes::E3035,
                    span,
                    "a slice is unsized and cannot be iterated by value",
                )
                .help("iterate a slice reference such as `*[T]` or `*mut [T]`")
                .emit();
                self.error_ty()
            }
            _ if self.is_error(ty) => ty,
            _ => {
                self.error(
                    codes::E3035,
                    span,
                    format!("cannot iterate over `{}`", self.ty_text(ty)),
                )
                .help("iterate an integer range, an array, `&array`, or a slice reference")
                .emit();
                self.error_ty()
            }
        }
    }
}
