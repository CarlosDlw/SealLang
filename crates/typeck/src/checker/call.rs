//! Call checking and generic instantiation.

use seal_def::{DefId, DefKind};
use seal_error_codes as codes;
use seal_hir::{ExprKind, HirExprId, Res};
use seal_span::Span;
use seal_ty::{Instance, TraitMethodDef, TraitRef, TyId};

use super::Checker;
use super::flow::Flow;

/// The checked pieces of one call to a trait function through a bound.
struct TraitFunctionCall<'a> {
    expr: HirExprId,
    function: DefId,
    trait_ref: Option<TraitRef>,
    args: &'a [HirExprId],
    explicit: Option<&'a [TyId]>,
    expected: Option<TyId>,
    span: Span,
}

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
        let (function, supplied_trait_ref) = match self.hir.expr(callee).kind.clone() {
            ExprKind::Path {
                res: Res::Function(definition),
                ..
            } => (Some(definition), None),
            ExprKind::Path {
                res:
                    Res::TraitFunction {
                        function,
                        trait_ref,
                    },
                ..
            } => (Some(function), Some(trait_ref)),
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
                (None, None)
            }
        };

        let Some(function) = function else {
            for arg in args {
                self.infer_expr(*arg, None, flow);
            }
            return self.error_ty();
        };

        if self.hir.definitions().kind(function) != DefKind::TraitMethod {
            let is_unsafe = self
                .results
                .signature(function)
                .is_some_and(|signature| signature.is_unsafe);
            let name = self.hir.definitions().name(function);
            if self.hir.extern_function(function).is_some() {
                self.require_unsafe_extern_call(span, is_unsafe, name);
            } else {
                self.require_unsafe_call(span, is_unsafe, name);
            }
        }

        let explicit = self
            .hir
            .expr_args(callee)
            .map(|substs| self.hir.ty_ctxt().substs_args(substs));
        if self.hir.definitions().kind(function) == DefKind::TraitMethod {
            return self.check_symbolic_trait_call(
                TraitFunctionCall {
                    expr,
                    function,
                    trait_ref: supplied_trait_ref,
                    args,
                    explicit: explicit.as_deref(),
                    expected,
                    span,
                },
                flow,
            );
        }
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

    /// Checks `T::function(...)` where the function is declared by a bound.
    ///
    /// The implementation is unknown until monomorphization, so the call
    /// records the trait method and its complete substitution instead of a
    /// concrete function.
    fn check_symbolic_trait_call(&mut self, call: TraitFunctionCall<'_>, flow: &mut Flow) -> TyId {
        let TraitFunctionCall {
            expr,
            function,
            trait_ref: supplied_trait_ref,
            args,
            explicit,
            expected,
            span,
        } = call;
        let Some(trait_def) = self.hir.definitions().get(function).parent else {
            self.error(
                codes::E2018,
                span,
                "this trait method has no declaring trait",
            )
            .emit();
            return self.error_ty();
        };
        let Some(declaration) = self.hir.trait_definition(trait_def) else {
            return self.error_ty();
        };
        let Some(method) = declaration
            .methods
            .iter()
            .find(|method| method.def == function)
            .cloned()
        else {
            return self.error_ty();
        };
        self.require_unsafe_call(span, method.is_unsafe, method.name);
        let Some(trait_ref) = supplied_trait_ref.or_else(|| self.visible_trait_ref(trait_def))
        else {
            for arg in args {
                self.infer_expr(*arg, None, flow);
            }
            return self.error_ty();
        };
        let Some(instantiated) =
            self.instantiate_trait_method(trait_ref, &method, explicit, expected)
        else {
            for arg in args {
                self.infer_expr(*arg, None, flow);
            }
            return self.error_ty();
        };
        match self.finish_trait_method_call(expr, instantiated, &method, args, flow, span) {
            Some((_, ret, _)) => ret,
            None => self.error_ty(),
        }
    }

    /// Finds the one visible bound that provides `trait_def`.
    ///
    /// A call through a type parameter is only meaningful when a single bound
    /// fixes the implementing type. Two bounds for the same trait leave the
    /// self type undecided, so the call is ambiguous.
    fn visible_trait_ref(&mut self, trait_def: DefId) -> Option<TraitRef> {
        let owner = self.owner?;
        let mut found: Vec<TraitRef> = Vec::new();
        for parameter in self.hir.flattened_generics(owner) {
            for trait_ref in self.visible_traits(parameter.param) {
                if trait_ref.trait_def == trait_def && !found.contains(&trait_ref) {
                    found.push(trait_ref);
                }
            }
        }
        match found.len() {
            1 => found.pop(),
            0 => {
                let name = self
                    .interner
                    .resolve(self.hir.definitions().name(trait_def));
                self.error(
                    codes::E3039,
                    self.hir.definitions().get(owner).span,
                    format!("the trait `{name}` is not visible in this body"),
                )
                .help("add the trait to the type parameter's bounds")
                .emit();
                None
            }
            _ => {
                let name = self
                    .interner
                    .resolve(self.hir.definitions().name(trait_def));
                self.error(
                    codes::E3040,
                    self.hir.definitions().get(owner).span,
                    format!("more than one bound provides the trait `{name}`"),
                )
                .help("remove one of the bounds, or call the function on a concrete type")
                .emit();
                None
            }
        }
    }

    /// Checks arguments and assembles one trait method call.
    ///
    /// `method.signature` excludes the receiver, so `params` are the trait and
    /// method parameters in declaration order. The complete substitution is
    /// recorded on `expr` for monomorphization.
    pub(super) fn finish_trait_method_call(
        &mut self,
        expr: HirExprId,
        instantiated: super::generics::Instantiated,
        method: &TraitMethodDef,
        args: &[HirExprId],
        flow: &mut Flow,
        span: Span,
    ) -> Option<(Vec<TyId>, TyId, seal_ty::SubstsId)> {
        if instantiated.params.len() != args.len() {
            self.error(
                codes::E3028,
                span,
                format!(
                    "method `{}` takes {} argument(s) but {} were supplied",
                    self.interner.resolve(method.name),
                    instantiated.params.len(),
                    args.len()
                ),
            )
            .help("pass exactly one argument for each parameter after the receiver")
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
        let args_ty = self.finish_arguments(&instantiated.generics, &instantiated.args, span)?;
        let substs = self.hir.ty_ctxt().substs(&args_ty);
        let params: Vec<TyId> = method
            .signature
            .params
            .iter()
            .map(|parameter| self.substitute(*parameter, substs))
            .collect();
        let ret = self.substitute(method.signature.ret, substs);
        self.results
            .record_instantiation(expr, Instance::new(method.def, substs));
        Some((params, ret, substs))
    }
}
