//! Calls through closure values.
//!
//! A concrete closure call has no lookup: the closure's checked signature
//! decides the arguments and the result, and its kind decides how the callee
//! must be adjusted. The adjustment is recorded with the call so MIR never
//! repeats the decision.

use seal_def::DefId;
use seal_error_codes as codes;
use seal_hir::HirExprId;
use seal_span::Span;
use seal_ty::{ClosureKind, TyId};

use super::Checker;
use super::flow::Flow;
use crate::results::ResolvedCall;

impl Checker<'_> {
    /// Checks a call of a concrete closure value.
    pub(super) fn check_closure_call(
        &mut self,
        expr: HirExprId,
        callee: HirExprId,
        closure: DefId,
        args: &[HirExprId],
        flow: &mut Flow,
        span: Span,
    ) -> TyId {
        let Some(info) = self.results.closure_info(closure).cloned() else {
            panic!(
                "closure {closure:?} was called before its body was checked; \
                 this is an internal compiler error"
            );
        };
        if info.kind == ClosureKind::FnMut
            && self.results.place(callee).is_some()
            && !self.borrow_is_writable(callee)
        {
            self.report_mutable_borrow(callee);
        }
        self.note_fn_once_call(callee, info.kind, flow);

        let params = info.signature.params.len();
        if params != args.len() {
            self.error(
                codes::E3008,
                span,
                format!(
                    "this closure takes {params} argument(s) but {} were supplied",
                    args.len()
                ),
            )
            .help("pass exactly one argument for each closure parameter")
            .emit();
        }
        for (index, arg) in args.iter().enumerate() {
            match info.signature.params.get(index) {
                Some(expected) => {
                    self.check_expr(*arg, *expected, flow);
                }
                None => {
                    self.infer_expr(*arg, None, flow);
                }
            }
        }

        self.results.record_call(
            expr,
            ResolvedCall::Closure {
                closure,
                kind: info.kind,
            },
        );
        info.signature.ret
    }
}
