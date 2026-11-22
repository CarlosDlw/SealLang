//! Control-flow and introspection built-ins.
//!
//! `panic` and `exit` diverge: their type is `!`, so they fit any expression
//! position. The equality assertions reuse the ordinary equality rules, which
//! read both operands without moving them. Introspection queries never evaluate
//! their type argument, but `type_of(value)` still checks and reads its operand
//! so a side effect in the value expression is not silently skipped.

use seal_ast::BinaryOp;
use seal_hir::HirExprId;
use seal_span::Span;
use seal_ty::TyId;

use super::Checker;
use super::flow::Flow;

impl Checker<'_> {
    /// Checks `assert_eq(left, right)` and `assert_ne(left, right)`.
    ///
    /// The operands follow exactly the same rules as `==` and `!=`: both are
    /// read, the left type selects the comparison, and the right operand must
    /// match it. The call itself produces `()`.
    pub(super) fn check_equality_assert(
        &mut self,
        lhs: HirExprId,
        rhs: HirExprId,
        span: Span,
        flow: &mut Flow,
    ) -> TyId {
        self.check_binary(BinaryOp::Eq, lhs, rhs, None, flow, span);
        self.unit()
    }

    /// Checks `panic(message)` and types it as `!`.
    pub(super) fn check_panic(&mut self, message: HirExprId, flow: &mut Flow) -> TyId {
        let expected = self.str_ref_ty();
        self.check_expr(message, expected, flow);
        self.never()
    }

    /// Checks `exit(code)` and types it as `!`.
    pub(super) fn check_exit(&mut self, code: HirExprId, flow: &mut Flow) -> TyId {
        let expected = self.i32_ty();
        self.check_expr(code, expected, flow);
        self.never()
    }

    /// Types `type_name::<T>()` as `*str`.
    pub(super) fn check_type_name(&self) -> TyId {
        self.str_ref_ty()
    }

    /// Checks `type_of(value)` and types it as `*str`.
    ///
    /// The operand is read, not moved, exactly like an equality operand. Its
    /// static type is printed at run time.
    pub(super) fn check_type_of(&mut self, operand: HirExprId, flow: &mut Flow) -> TyId {
        let _ = self.infer_expr_mode(operand, None, flow, super::PlaceMode::Borrow);
        self.str_ref_ty()
    }

    /// Checks a `size_of` or `align_of` query and types it as `usize`.
    ///
    /// The queried type must be sized and its layout must fit the target address
    /// space. `storage_ty` already applies exactly those rules and reports the
    /// established unsized-value and oversized-array diagnostics.
    pub(super) fn check_type_layout(&mut self, ty: TyId, span: Span, query: &str) -> TyId {
        if self.is_error(ty) {
            return self.error_ty();
        }
        self.storage_ty(ty, span, query);
        self.usize_ty()
    }
}
