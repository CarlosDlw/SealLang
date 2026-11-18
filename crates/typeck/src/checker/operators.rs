//! Unary and binary operator checking.

use seal_ast::{AssignOp, BinaryOp, UnaryOp};
use seal_hir::{ExprKind, HirExprId};
use seal_span::Span;
use seal_ty::TyId;

use super::Checker;
use super::PlaceMode;
use super::flow::Flow;

impl Checker<'_> {
    pub(super) fn check_unary(
        &mut self,
        op: UnaryOp,
        operand: HirExprId,
        expected: Option<TyId>,
        flow: &mut Flow,
        span: Span,
    ) -> TyId {
        match op {
            UnaryOp::Neg => {
                let hint = expected.filter(|ty| self.is_numeric(*ty));
                let ty = self.operand_ty(operand, hint, flow, true);
                if self.is_error(ty) {
                    return ty;
                }
                if !self.is_numeric(ty) {
                    self.report_operand(op.as_str(), ty, self.hir.expr(operand).span, "numeric");
                    return self.error_ty();
                }
                if self.is_integer(ty) && self.is_unsigned(ty) {
                    self.error(
                        seal_error_codes::E3002,
                        span,
                        format!("cannot negate a value of type `{}`", self.ty_text(ty)),
                    )
                    .help("negation requires a signed integer or a floating-point value")
                    .emit();
                    return self.error_ty();
                }
                ty
            }
            UnaryOp::Not => {
                let ty = self.operand_ty(operand, Some(self.bool_ty()), flow, false);
                if !self.is_bool(ty) && !self.is_error(ty) && !self.is_never(ty) {
                    self.error(
                        seal_error_codes::E3002,
                        self.hir.expr(operand).span,
                        format!("`!` requires a `bool`, found `{}`", self.ty_text(ty)),
                    )
                    .help("logical negation only applies to `bool`")
                    .emit();
                    return self.error_ty();
                }
                self.bool_ty()
            }
            UnaryOp::BitNot => {
                let hint = expected.filter(|ty| self.is_integer(*ty));
                let ty = self.operand_ty(operand, hint, flow, false);
                if !self.is_integer(ty) && !self.is_error(ty) && !self.is_never(ty) {
                    self.report_operand(op.as_str(), ty, self.hir.expr(operand).span, "integer");
                    return self.error_ty();
                }
                ty
            }
        }
    }

    pub(super) fn check_binary(
        &mut self,
        op: BinaryOp,
        lhs: HirExprId,
        rhs: HirExprId,
        expected: Option<TyId>,
        flow: &mut Flow,
        _span: Span,
    ) -> TyId {
        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
                self.numeric_binary(op.as_str(), lhs, rhs, expected, flow, false)
            }
            BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
                self.numeric_binary(op.as_str(), lhs, rhs, expected, flow, true)
            }
            BinaryOp::Shl | BinaryOp::Shr => {
                let hint = expected.filter(|ty| self.is_integer(*ty));
                let lhs_ty = self.infer_expr(lhs, hint, flow);
                if !self.is_integer(lhs_ty) && !self.is_error(lhs_ty) {
                    self.report_operand(op.as_str(), lhs_ty, self.hir.expr(lhs).span, "integer");
                    self.infer_expr(rhs, None, flow);
                    return self.error_ty();
                }
                self.check_expr(rhs, lhs_ty, flow);
                lhs_ty
            }
            BinaryOp::Eq | BinaryOp::Ne => {
                // Equality reads both operands; it never moves them out of the
                // place they came from. The read mode lets a non-`Copy` operand
                // be a pointee or a static without reporting a move that the
                // comparison does not perform.
                let lhs_ty = self.infer_expr_mode(lhs, None, flow, PlaceMode::Borrow);
                let lhs_span = self.hir.expr(lhs).span;
                if let seal_ty::TyKind::Param(param) = self.hir.ty_ctxt().kind(lhs_ty) {
                    self.report_parameter_operation(op.as_str(), param, lhs_span);
                } else if !self.is_error(lhs_ty) {
                    self.check_comparable(op.as_str(), lhs_ty, lhs_span);
                }
                self.check_expr_read(rhs, lhs_ty, flow);
                self.bool_ty()
            }
            BinaryOp::Lt | BinaryOp::Gt | BinaryOp::Le | BinaryOp::Ge => {
                let lhs_ty = self.infer_expr(lhs, None, flow);
                if !self.is_numeric(lhs_ty) && !self.is_char(lhs_ty) && !self.is_error(lhs_ty) {
                    self.report_operand(op.as_str(), lhs_ty, self.hir.expr(lhs).span, "ordered");
                    self.infer_expr(rhs, None, flow);
                    return self.error_ty();
                }
                self.check_expr(rhs, lhs_ty, flow);
                self.bool_ty()
            }
            BinaryOp::And | BinaryOp::Or => {
                let bool_ty = self.bool_ty();
                let lhs_ty = self.infer_expr(lhs, Some(bool_ty), flow);
                let mut valid = true;
                if !self.is_bool(lhs_ty) && !self.is_error(lhs_ty) && !self.is_never(lhs_ty) {
                    self.report_logical(op.as_str(), lhs_ty, self.hir.expr(lhs).span);
                    valid = false;
                }
                let rhs_ty = self.infer_expr(rhs, Some(bool_ty), flow);
                if !self.is_bool(rhs_ty) && !self.is_error(rhs_ty) && !self.is_never(rhs_ty) {
                    self.report_logical(op.as_str(), rhs_ty, self.hir.expr(rhs).span);
                    valid = false;
                }
                if valid || self.is_error(lhs_ty) || self.is_error(rhs_ty) {
                    bool_ty
                } else {
                    self.error_ty()
                }
            }
        }
    }

    fn numeric_binary(
        &mut self,
        op: &str,
        lhs: HirExprId,
        rhs: HirExprId,
        expected: Option<TyId>,
        flow: &mut Flow,
        integer_only: bool,
    ) -> TyId {
        let hint = expected.filter(|ty| {
            if integer_only {
                self.is_integer(*ty)
            } else {
                self.is_numeric(*ty)
            }
        });
        let lhs_ty = self.infer_expr(lhs, hint, flow);
        if self.is_error(lhs_ty) {
            self.infer_expr(rhs, None, flow);
            return lhs_ty;
        }
        let valid = if integer_only {
            self.is_integer(lhs_ty)
        } else {
            self.is_numeric(lhs_ty)
        };
        if !valid {
            let requirement = if integer_only { "integer" } else { "numeric" };
            self.report_operand(op, lhs_ty, self.hir.expr(lhs).span, requirement);
            self.infer_expr(rhs, None, flow);
            return self.error_ty();
        }
        self.check_expr(rhs, lhs_ty, flow);
        lhs_ty
    }

    /// Infers an operand, allowing the most negative signed literal.
    fn operand_ty(
        &mut self,
        operand: HirExprId,
        expected: Option<TyId>,
        flow: &mut Flow,
        allow_signed_min: bool,
    ) -> TyId {
        if let ExprKind::Lit(literal) = self.hir.expr(operand).kind.clone() {
            let span = self.hir.expr(operand).span;
            let ty = self.literal_ty(literal, expected, span, allow_signed_min);
            self.results.record_expr(operand, ty);
            ty
        } else {
            self.infer_expr(operand, expected, flow)
        }
    }

    fn is_unsigned(&self, ty: TyId) -> bool {
        matches!(self.hir.ty_ctxt().kind(ty), seal_ty::TyKind::Uint(_))
    }

    fn report_operand(&self, op: &str, ty: TyId, span: Span, requirement: &str) {
        if let seal_ty::TyKind::Param(param) = self.hir.ty_ctxt().kind(ty) {
            self.report_parameter_operation(op, param, span);
            return;
        }
        self.error(
            seal_error_codes::E3002,
            span,
            format!(
                "operator `{op}` cannot be applied to type `{}`",
                self.ty_text(ty)
            ),
        )
        .help(format!(
            "{requirement} operands are required for this operator"
        ))
        .emit();
    }

    fn report_parameter_operation(&self, op: &str, param: seal_ty::ParamTyId, span: Span) {
        let name = self
            .interner
            .resolve(self.hir.ty_ctxt().param_data(param).name);
        self.error(
            seal_error_codes::E3041,
            span,
            format!("operator `{op}` is not available for the type parameter `{name}`"),
        )
        .help("add a bound that provides the operation, or make the function concrete")
        .emit();
    }

    fn report_logical(&self, op: &str, ty: TyId, span: Span) {
        self.error(
            seal_error_codes::E3002,
            span,
            format!(
                "logical operator `{op}` requires `bool`, found `{}`",
                self.ty_text(ty)
            ),
        )
        .help("`&&` and `||` only combine `bool` values")
        .emit();
    }

    pub(super) fn check_assign_op(&self, op: AssignOp, ty: TyId, span: Span) -> bool {
        if self.is_error(ty) {
            return true;
        }
        if let seal_ty::TyKind::Param(param) = self.hir.ty_ctxt().kind(ty) {
            let name = self
                .interner
                .resolve(self.hir.ty_ctxt().param_data(param).name);
            self.error(
                seal_error_codes::E3041,
                span,
                format!(
                    "compound assignment `{}` is not available for the type parameter `{name}`",
                    op.as_str()
                ),
            )
            .help("add a bound that provides the operation, or make the function concrete")
            .emit();
            return false;
        }
        let (valid, requirement) = match op {
            AssignOp::Assign => return true,
            AssignOp::AddAssign
            | AssignOp::SubAssign
            | AssignOp::MulAssign
            | AssignOp::DivAssign
            | AssignOp::RemAssign => (self.is_numeric(ty), "numeric"),
            AssignOp::ShlAssign
            | AssignOp::ShrAssign
            | AssignOp::BitAndAssign
            | AssignOp::BitOrAssign
            | AssignOp::BitXorAssign => (self.is_integer(ty), "integer"),
        };
        if valid {
            return true;
        }
        self.error(
            seal_error_codes::E3002,
            span,
            format!(
                "compound assignment `{}` cannot be applied to `{}`",
                op.as_str(),
                self.ty_text(ty)
            ),
        )
        .help(format!(
            "{requirement} places are required for this operator"
        ))
        .emit();
        false
    }
}
