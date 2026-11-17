//! Call checking and generic instantiation.

use seal_error_codes as codes;
use seal_hir::{ExprKind, HirExprId, Res};
use seal_span::Span;
use seal_ty::TyId;

use super::Checker;
use super::flow::Flow;

impl Checker<'_> {
    pub(super) fn check_call(
        &mut self,
        expr: HirExprId,
        callee: HirExprId,
        args: &[HirExprId],
        expected: Option<TyId>,
        flow: &mut Flow,
        span: Span,
    ) -> TyId {
        let function = match self.hir.expr(callee).kind.clone() {
            ExprKind::Path {
                res: Res::Function(definition),
                ..
            } => Some(definition),
            _ => {
                let ty = self.infer_expr(callee, None, flow);
                if !self.is_error(ty) {
                    self.error(
                        codes::E3006,
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

        let explicit = self
            .hir
            .expr_args(callee)
            .map(|substs| self.hir.ty_ctxt().substs_args(substs));
        let from_impl = self.impl_of_function(function).is_some();
        let seed = if from_impl {
            match self.associated_seed(function, explicit.as_deref(), expected, span) {
                Some(seed) => seed,
                None => {
                    for arg in args {
                        self.infer_expr(*arg, None, flow);
                    }
                    return self.error_ty();
                }
            }
        } else {
            Default::default()
        };
        let explicit_own = (!from_impl).then_some(explicit.as_deref()).flatten();
        let instantiated = self.instantiate_function(function, &seed, explicit_own, expected);

        if instantiated.params.len() != args.len() {
            self.error(
                codes::E3008,
                span,
                format!(
                    "this function takes {} argument(s) but {} were supplied",
                    instantiated.params.len(),
                    args.len()
                ),
            )
            .help("pass exactly one argument for each parameter")
            .emit();
        }

        for (index, arg) in args.iter().enumerate() {
            match instantiated.params.get(index) {
                Some(expected) => self.check_expr(*arg, *expected, flow),
                None => {
                    self.infer_expr(*arg, None, flow);
                }
            }
        }

        match self.finish_function_instantiation(expr, function, instantiated, span) {
            Some((_, ret, _)) => ret,
            None => self.error_ty(),
        }
    }
}
