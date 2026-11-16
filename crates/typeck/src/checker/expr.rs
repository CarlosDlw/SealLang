//! Expression checking.

use seal_hir::{ExprKind, HirBlockId, HirExprId, Res};
use seal_span::Span;
use seal_ty::TyId;

use super::Checker;
use super::flow::Flow;

impl Checker<'_> {
    /// Checks `expr` against `expected`, reporting a mismatch at the expression.
    pub(super) fn check_expr(&mut self, expr: HirExprId, expected: TyId, flow: &mut Flow) {
        let found = self.infer_expr(expr, Some(expected), flow);
        self.mismatch(self.hir.expr(expr).span, expected, found);
    }

    /// Computes the type of `expr`, using `expected` as a hint where it changes
    /// inference (literals and conditional branches).
    pub(super) fn infer_expr(
        &mut self,
        expr: HirExprId,
        expected: Option<TyId>,
        flow: &mut Flow,
    ) -> TyId {
        let (span, kind) = {
            let expr = self.hir.expr(expr);
            (expr.span, expr.kind.clone())
        };

        let ty = match kind {
            ExprKind::Lit(literal) => self.literal_ty(literal, expected, span, false),
            ExprKind::Path(res) => self.path_ty(res, span, flow),
            ExprKind::Unary { op, operand } => self.check_unary(op, operand, expected, flow, span),
            ExprKind::Binary { op, lhs, rhs } => {
                self.check_binary(op, lhs, rhs, expected, flow, span)
            }
            ExprKind::Assign { op, target, value } => {
                self.check_assign(op, target, value, flow, span)
            }
            ExprKind::Update {
                op,
                target,
                postfix: _,
            } => self.check_update(op, target, flow, span),
            ExprKind::Call { callee, args } => self.check_call(callee, &args, flow, span),
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.check_if(condition, then_branch, else_branch, expected, flow, span),
            ExprKind::Error => self.error_ty(),
        };

        self.results.record_expr(expr, ty);
        ty
    }

    fn path_ty(&mut self, res: Res, span: Span, flow: &mut Flow) -> TyId {
        match res {
            Res::Local(local) => {
                let ty = self.local_type(local);
                if !flow.diverges && !flow.is_initialized(local) && !self.is_error(ty) {
                    self.report_uninitialized(local, span);
                }
                ty
            }
            Res::Function(_) => {
                self.report_function_as_value(span);
                self.error_ty()
            }
        }
    }

    /// Checks that `condition` has type `bool`.
    pub(super) fn check_condition(&mut self, condition: HirExprId, flow: &mut Flow) {
        let ty = self.infer_expr(condition, Some(self.bool_ty()), flow);
        if !self.is_bool(ty) && !self.is_error(ty) && !self.is_never(ty) {
            self.error(
                seal_error_codes::E3003,
                self.hir.expr(condition).span,
                format!("expected `bool`, found `{}`", self.ty_text(ty)),
            )
            .help("conditions must have type `bool`")
            .emit();
        }
    }

    fn check_if(
        &mut self,
        condition: HirExprId,
        then_branch: HirBlockId,
        else_branch: Option<HirBlockId>,
        expected: Option<TyId>,
        flow: &mut Flow,
        span: Span,
    ) -> TyId {
        self.check_condition(condition, flow);

        let Some(else_branch) = else_branch else {
            let otherwise = flow.clone();
            let then_out = self.check_block(then_branch, None, flow.clone());
            let result = match expected {
                Some(expected) if expected != self.unit() && !self.is_error(expected) => {
                    self.mismatch(span, expected, self.unit());
                    if self.is_error(then_out.ty) {
                        self.error_ty()
                    } else {
                        expected
                    }
                }
                Some(expected) => {
                    if !self.is_unit(then_out.ty)
                        && !self.is_never(then_out.ty)
                        && !self.is_error(then_out.ty)
                    {
                        self.report_if_without_else(then_branch, then_out.ty);
                    }
                    expected
                }
                None => {
                    if !self.is_unit(then_out.ty)
                        && !self.is_never(then_out.ty)
                        && !self.is_error(then_out.ty)
                    {
                        self.report_if_without_else(then_branch, then_out.ty);
                    }
                    self.unit()
                }
            };
            flow.merge(then_out.flow, otherwise);
            return result;
        };

        let then_out = self.check_block(then_branch, expected, flow.clone());
        let else_expected = match expected {
            Some(expected) => Some(expected),
            None if self.is_never(then_out.ty) => None,
            None => Some(then_out.ty),
        };
        let else_out = self.check_block(else_branch, else_expected, flow.clone());
        flow.merge(then_out.flow, else_out.flow);

        if let Some(expected) = expected {
            if self.is_error(then_out.ty) || self.is_error(else_out.ty) {
                return self.error_ty();
            }
            return expected;
        }

        if self.is_never(then_out.ty) {
            else_out.ty
        } else if self.is_never(else_out.ty) {
            then_out.ty
        } else if self.is_error(then_out.ty) || self.is_error(else_out.ty) {
            self.error_ty()
        } else if then_out.ty == else_out.ty {
            then_out.ty
        } else {
            // `check_block` already reported the mismatch against `then_out.ty`.
            self.error_ty()
        }
    }

    fn report_if_without_else(&self, then_branch: HirBlockId, found: TyId) {
        self.error(
            seal_error_codes::E3001,
            self.hir.block(then_branch).span,
            format!(
                "`if` without `else` requires the branch to have type `()`, found `{}`",
                self.ty_text(found)
            ),
        )
        .help("add an `else` branch, or make the branch evaluate to `()`")
        .emit();
    }
}
