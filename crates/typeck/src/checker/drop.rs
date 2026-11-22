//! The `drop(value)` built-in.
//!
//! `drop` releases its operand at the point of the call. It is not a function:
//! type checking accepts any operand type and returns `()`, and MIR lowering
//! emits a `Drop` statement with [`DropKind::Explicit`], which consumes a
//! non-`Copy` place and does nothing for a `Copy` value. That split keeps the
//! move decision in one place, next to every other move.
//!
//! [`DropKind::Explicit`]: seal_mir::DropKind::Explicit

use seal_hir::HirExprId;
use seal_span::Span;
use seal_ty::TyId;

use super::Checker;
use super::flow::Flow;

impl Checker<'_> {
    /// Checks `drop(value)` and returns `()`.
    ///
    /// The operand is inferred in place mode: it has to denote storage for a
    /// non-`Copy` value to be moved out of, and reading through a reference
    /// would move out of borrowed storage, which `E3018` rejects here rather
    /// than later in borrow checking.
    pub(super) fn check_drop(&mut self, operand: HirExprId, span: Span, flow: &mut Flow) -> TyId {
        self.check_consumed_operand(operand, span, flow);
        self.hir.ty_ctxt().unit_ty()
    }

    /// Checks an operand consumed by value and returns its type.
    ///
    /// `drop` and `mem::forget` share the same ownership rules: the operand is
    /// inferred once in place mode, and a non-`Copy` value cannot be moved out
    /// of a reference. The caller decides what happens after consumption.
    pub(super) fn check_consumed_operand(
        &mut self,
        operand: HirExprId,
        span: Span,
        flow: &mut Flow,
    ) -> TyId {
        let ty = self.infer_expr(operand, None, flow);
        if self.is_error(ty) {
            return self.error_ty();
        }
        let moves_out_of_reference = self
            .results
            .place(operand)
            .is_some_and(|place| place.contains_deref())
            && !self.is_copy(ty);
        if moves_out_of_reference {
            self.report_move_out_of_reference(ty, span);
        }
        ty
    }
}
