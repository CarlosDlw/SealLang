//! Compile-time calls to `const func` instances.

use seal_mir::{CallTarget, Operand};
use seal_span::Span;
use seal_ty::{Instance, TyKind};

use super::*;

impl<'a> Interpreter<'a> {
    pub(super) fn eval_call(
        &mut self,
        target: CallTarget,
        args: &[Operand],
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        let instance = match target {
            CallTarget::Direct { instance, .. } => instance,
            // The body lives in C, not in the MIR the evaluator can execute.
            CallTarget::Extern { .. } => {
                return Err(self.error(EvalErrorKind::ExternCall, span));
            }
            CallTarget::TraitMethod { .. } => {
                return Err(self.error(
                    EvalErrorKind::DynamicDispatch {
                        const_fn: self.caller_is_const_fn(),
                    },
                    span,
                ));
            }
        };
        self.instantiate_call(instance, args, span)
    }

    fn instantiate_call(
        &mut self,
        instance: Instance,
        args: &[Operand],
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        let body = self.mir.body_of(instance.def).cloned().ok_or_else(|| {
            self.error(
                EvalErrorKind::Internal("callee has no MIR body".into()),
                span,
            )
        })?;
        if body.kind != BodyKind::Function || !body.is_const {
            return Err(self.error(
                EvalErrorKind::NonConstFunction {
                    const_fn: self.caller_is_const_fn(),
                },
                span,
            ));
        }
        if self.tcx.substs_args(instance.args).iter().any(|ty| {
            self.tcx.contains_param(*ty) || matches!(self.tcx.kind(*ty), TyKind::Infer(_))
        }) {
            return Err(self.error(EvalErrorKind::NonConcreteArguments, span));
        }
        let mut values = Vec::with_capacity(args.len());
        for argument in args {
            values.push(self.read_operand(argument, span)?);
        }
        if values.len() != body.arg_count {
            return Err(self.error(
                EvalErrorKind::Internal(format!(
                    "call passes {} argument(s) but the callee takes {}",
                    values.len(),
                    body.arg_count
                )),
                span,
            ));
        }
        let specialized = seal_mir::instantiate_body(self.hir, &body, instance);
        self.eval_body_frame(instance.def, instance, &specialized, &values)
    }

    pub(super) fn caller_is_const_fn(&self) -> bool {
        self.current_owner()
            .and_then(|def| self.hir.body_of(def))
            .is_some_and(|body| body.is_const)
    }
}
