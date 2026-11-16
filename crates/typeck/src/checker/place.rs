//! Assignment, compound assignment, and increment/decrement.

use seal_ast::{AssignOp, UpdateOp};
use seal_hir::HirExprId;
use seal_span::Span;
use seal_ty::TyId;

use super::Checker;
use super::flow::Flow;

impl Checker<'_> {
    pub(super) fn check_assign(
        &mut self,
        op: AssignOp,
        target: HirExprId,
        value: HirExprId,
        flow: &mut Flow,
        span: Span,
    ) -> TyId {
        let target_span = self.hir.expr(target).span;
        let Some(local) = self.local_place(target) else {
            self.report_bad_target(target, target_span);
            self.results.record_expr(target, self.error_ty());
            self.infer_expr(value, None, flow);
            return self.error_ty();
        };

        let ty = self.local_type(local);
        self.results.record_expr(target, ty);
        let plain = op == AssignOp::Assign;
        let mutable = self.hir.definitions().is_mutable(local);
        // D9: an uninitialized immutable binding may be initialized once by
        // assignment. Once it holds a value, a write needs `let mut`.
        if !mutable && (!plain || flow.is_initialized(local)) {
            let name = self.def_name(local);
            self.error(
                seal_error_codes::E3004,
                target_span,
                format!("cannot assign to `{name}`, as it is not declared as mutable"),
            )
            .help("declare it with `let mut` to allow assignment")
            .emit();
        }
        if !plain {
            if !flow.is_initialized(local) && !flow.diverges {
                self.report_uninitialized(local, target_span);
            }
            self.check_assign_op(op, ty, span);
        }

        self.check_expr(value, ty, flow);
        flow.mark_initialized(local);
        self.unit()
    }

    pub(super) fn check_update(
        &mut self,
        op: UpdateOp,
        target: HirExprId,
        flow: &mut Flow,
        span: Span,
    ) -> TyId {
        let target_span = self.hir.expr(target).span;
        let Some(local) = self.local_place(target) else {
            self.report_bad_target(target, target_span);
            self.results.record_expr(target, self.error_ty());
            return self.error_ty();
        };

        let ty = self.local_type(local);
        self.results.record_expr(target, ty);
        if !self.hir.definitions().is_mutable(local) {
            let name = self.def_name(local);
            self.error(
                seal_error_codes::E3004,
                target_span,
                format!("cannot assign to `{name}`, as it is not declared as mutable"),
            )
            .help("declare it with `let mut` to allow assignment")
            .emit();
        }
        if !flow.is_initialized(local) && !flow.diverges {
            self.report_uninitialized(local, target_span);
        }
        if !self.is_numeric(ty) && !self.is_error(ty) {
            self.error(
                seal_error_codes::E3002,
                span,
                format!(
                    "`{}` requires a numeric place, found `{}`",
                    op.as_str(),
                    self.ty_text(ty)
                ),
            )
            .help("increment and decrement work on integer and floating-point places")
            .emit();
        }

        flow.mark_initialized(local);
        self.unit()
    }

    fn report_bad_target(&self, target: HirExprId, span: Span) {
        if self.is_function_target(target) {
            self.error(seal_error_codes::E3005, span, "cannot assign to a function")
                .help("only local bindings are assignable")
                .emit();
        } else if !self.is_error_target(target) {
            self.error(seal_error_codes::E3005, span, "invalid assignment target")
                .help("assign to a local binding")
                .emit();
        }
    }

    fn is_function_target(&self, target: HirExprId) -> bool {
        matches!(
            self.hir.expr(target).kind,
            seal_hir::ExprKind::Path(seal_hir::Res::Function(_))
        )
    }

    fn is_error_target(&self, target: HirExprId) -> bool {
        matches!(self.hir.expr(target).kind, seal_hir::ExprKind::Error)
    }
}
