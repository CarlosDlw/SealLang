//! Call checking.

use seal_hir::{ExprKind, HirExprId, Res};
use seal_span::Span;
use seal_ty::TyId;

use super::Checker;
use super::flow::Flow;

impl Checker<'_> {
    pub(super) fn check_call(
        &mut self,
        callee: HirExprId,
        args: &[HirExprId],
        flow: &mut Flow,
        span: Span,
    ) -> TyId {
        let function = match self.hir.expr(callee).kind.clone() {
            ExprKind::Path(Res::Function(definition)) => Some(definition),
            _ => {
                let ty = self.infer_expr(callee, None, flow);
                if !self.is_error(ty) {
                    self.error(
                        seal_error_codes::E3006,
                        self.hir.expr(callee).span,
                        "this expression is not callable",
                    )
                    .help("only function items can be called")
                    .emit();
                }
                None
            }
        };

        let Some(function) = function else {
            for arg in args {
                self.infer_expr(*arg, None, flow);
            }
            return self.error_ty();
        };

        let signature = self
            .results
            .signature(function)
            .expect("every function has a signature")
            .clone();
        if signature.params.len() != args.len() {
            self.error(
                seal_error_codes::E3008,
                span,
                format!(
                    "this function takes {} argument(s) but {} were supplied",
                    signature.params.len(),
                    args.len()
                ),
            )
            .help("pass exactly one argument for each parameter")
            .emit();
        }

        for (index, arg) in args.iter().enumerate() {
            match signature.params.get(index) {
                Some(expected) => self.check_expr(*arg, *expected, flow),
                None => {
                    self.infer_expr(*arg, None, flow);
                }
            }
        }

        signature.ret
    }
}
