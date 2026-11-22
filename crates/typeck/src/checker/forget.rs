//! The `mem::forget(value)` intrinsic.
//!
//! `forget` consumes its operand exactly like `drop`, but it runs no
//! destructor. The consumption rules are shared with `drop`: a non-`Copy` place
//! reached through a reference cannot be moved, and the ordinary move/static
//! checks apply. The public `core::mem::forget` wrapper is not a constant
//! function, so a direct intrinsic use in a constant context is rejected here
//! because lowering bypasses the ordinary call check.

use seal_hir::HirExprId;
use seal_span::Span;
use seal_ty::TyId;

use super::Checker;
use super::flow::Flow;

impl Checker<'_> {
    /// Checks `mem::forget(value)` and returns `()`.
    pub(super) fn check_forget(&mut self, operand: HirExprId, span: Span, flow: &mut Flow) -> TyId {
        let ty = self.check_consumed_operand(operand, span, flow);
        if !self.is_error(ty) && self.in_const_context() {
            self.report_non_const_call(span);
        }
        self.hir.ty_ctxt().unit_ty()
    }
}
