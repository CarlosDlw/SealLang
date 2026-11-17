//! Generic signature instantiation, inference, and bound checking.

use seal_def::{DefId, DefKind};
use seal_error_codes as codes;
use seal_hash::FxHashMap;
use seal_span::Span;
use seal_ty::{
    AdtId, Bound, GenericParamDef, Instance, ParamTyId, SubstsId, TraitRef, TyId, TyKind,
};

use super::Checker;
use super::trait_solver::Proof;

/// A signature with fresh or pre-matched arguments substituted.
pub(super) struct Instantiated {
    /// One argument per flattened generic parameter.
    pub(super) args: Vec<TyId>,
    /// The flattened generic parameter declarations.
    pub(super) generics: Vec<GenericParamDef>,
    /// Parameter types after substitution.
    pub(super) params: Vec<TyId>,
}

impl Checker<'_> {
    /// Creates one type argument for each visible parameter.
    ///
    /// Parameters already bound by an impl header use the matched type.
    /// Explicit method arguments fill the method's own parameters. Everything
    /// else becomes a fresh inference variable.
    pub(super) fn instantiate_arguments(
        &mut self,
        generics: &[GenericParamDef],
        seed: &FxHashMap<ParamTyId, TyId>,
        explicit_own: Option<&[TyId]>,
        own_start: usize,
    ) -> Vec<TyId> {
        let mut args = Vec::with_capacity(generics.len());
        for (index, parameter) in generics.iter().enumerate() {
            if let Some(argument) = seed.get(&parameter.param).copied() {
                args.push(argument);
                continue;
            }
            if index >= own_start
                && let Some(explicit) = explicit_own
                && let Some(argument) = explicit.get(index - own_start).copied()
            {
                args.push(argument);
                continue;
            }
            args.push(self.infer.fresh_ty_var(self.hir.ty_ctxt()));
        }
        args
    }

    /// Instantiates a function's declared signature.
    pub(super) fn instantiate_function(
        &mut self,
        function: DefId,
        seed: &FxHashMap<ParamTyId, TyId>,
        explicit_own: Option<&[TyId]>,
        expected: Option<TyId>,
    ) -> Instantiated {
        let signature = self
            .results
            .signature(function)
            .expect("every function has a signature")
            .clone();
        let own_count = self
            .hir
            .generics(function)
            .map(|generics| generics.params.len())
            .unwrap_or(0);
        let own_start = signature.generics.len().saturating_sub(own_count);
        let args = self.instantiate_arguments(&signature.generics, seed, explicit_own, own_start);
        let substs = self.hir.ty_ctxt().substs(&args);
        let params = signature
            .params
            .iter()
            .map(|parameter| self.hir.ty_ctxt().substitute(*parameter, substs))
            .collect();
        let ret = self.hir.ty_ctxt().substitute(signature.ret, substs);
        if let Some(expected) = expected {
            let _ = self.infer.unify(self.hir.ty_ctxt(), expected, ret);
        }
        Instantiated {
            args,
            generics: signature.generics,
            params,
        }
    }

    /// Instantiates a trait method through a concrete trait reference.
    pub(super) fn instantiate_trait_method(
        &mut self,
        trait_ref: TraitRef,
        method: &seal_ty::TraitMethodDef,
        explicit_own: Option<&[TyId]>,
        expected: Option<TyId>,
    ) -> Option<Instantiated> {
        let declaration = self.hir.trait_definition(trait_ref.trait_def)?;
        let mut generics = declaration.generics.params.clone();
        generics.extend(method.generics.params.iter().cloned());
        let mut seed = FxHashMap::default();
        seed.insert(generics.first()?.param, trait_ref.self_ty);
        let trait_args = self.hir.ty_ctxt().substs_args(trait_ref.args);
        let own_start = declaration.generics.params.len();
        for (parameter, argument) in declaration.generics.params.iter().skip(1).zip(trait_args) {
            seed.insert(parameter.param, argument);
        }
        let args = self.instantiate_arguments(&generics, &seed, explicit_own, own_start);
        let substs = self.hir.ty_ctxt().substs(&args);
        let params = method
            .signature
            .params
            .iter()
            .map(|parameter| self.hir.ty_ctxt().substitute(*parameter, substs))
            .collect();
        let ret = self.hir.ty_ctxt().substitute(method.signature.ret, substs);
        if let Some(expected) = expected {
            let _ = self.infer.unify(self.hir.ty_ctxt(), expected, ret);
        }
        Some(Instantiated {
            args,
            generics,
            params,
        })
    }

    /// Resolves, checks, and records one complete function instantiation.
    ///
    /// Returns the instantiated parameter and return types plus the complete
    /// substitution. `None` means a diagnostic made the call unusable.
    pub(super) fn finish_function_instantiation(
        &mut self,
        expr: seal_hir::HirExprId,
        function: DefId,
        instantiated: Instantiated,
        span: Span,
    ) -> Option<(Vec<TyId>, TyId, SubstsId)> {
        let args = self.finish_arguments(&instantiated.generics, &instantiated.args, span)?;
        let substs = self.hir.ty_ctxt().substs(&args);
        let signature = self
            .results
            .signature(function)
            .expect("every function has a signature")
            .clone();
        let params = signature
            .params
            .iter()
            .map(|parameter| self.hir.ty_ctxt().substitute(*parameter, substs))
            .collect();
        let ret = self.hir.ty_ctxt().substitute(signature.ret, substs);
        self.results
            .record_instantiation(expr, Instance::new(function, substs));
        Some((params, ret, substs))
    }

    /// Resolves arguments, reports unbound parameters, and checks bounds.
    ///
    /// `None` means at least one diagnostic was emitted.
    pub(super) fn finish_arguments(
        &mut self,
        generics: &[GenericParamDef],
        args: &[TyId],
        span: Span,
    ) -> Option<Vec<TyId>> {
        let resolved: Vec<TyId> = args
            .iter()
            .map(|argument| self.resolve_deep(*argument))
            .collect();
        let unresolved: Vec<&GenericParamDef> = generics
            .iter()
            .zip(&resolved)
            .filter_map(|(parameter, argument)| self.contains_infer(*argument).then_some(parameter))
            .collect();
        if !unresolved.is_empty() {
            self.report_unresolved(&unresolved, span);
            return None;
        }
        let substs = self.hir.ty_ctxt().substs(&resolved);
        let mut valid = true;
        for (parameter, argument) in generics.iter().zip(&resolved) {
            if self.is_error(*argument) {
                continue;
            }
            for bound in &parameter.bounds {
                if !self.check_bound(parameter, *argument, bound, substs, span) {
                    valid = false;
                }
            }
        }
        valid.then_some(resolved)
    }

    /// Resolves and records one generic aggregate literal.
    pub(super) fn finish_aggregate_instantiation(
        &mut self,
        expr: seal_hir::HirExprId,
        definition: DefId,
        generics: &[GenericParamDef],
        args: &[TyId],
        span: Span,
    ) -> Option<Vec<TyId>> {
        let args = self.finish_arguments(generics, args, span)?;
        let substs = self.hir.ty_ctxt().substs(&args);
        self.results
            .record_instantiation(expr, Instance::new(definition, substs));
        Some(args)
    }

    /// Chooses the type arguments of an aggregate literal.
    ///
    /// Explicit arguments win. Otherwise an expected instantiation of the same
    /// aggregate supplies every argument, and the remaining parameters become
    /// fresh variables inferred from the fields.
    pub(super) fn aggregate_arguments(
        &mut self,
        adt: AdtId,
        generics: &[GenericParamDef],
        explicit: Option<SubstsId>,
        expected: Option<TyId>,
    ) -> Vec<TyId> {
        if let Some(explicit) = explicit {
            let args = self.hir.ty_ctxt().substs_args(explicit);
            if args.len() == generics.len() {
                return args;
            }
        }
        if let Some(expected) = expected
            && let TyKind::Adt {
                adt: expected_adt,
                args,
            } = self.hir.ty_ctxt().kind(expected)
            && expected_adt == adt
        {
            let args = self.hir.ty_ctxt().substs_args(args);
            if args.len() == generics.len() {
                return args;
            }
        }
        self.instantiate_arguments(generics, &FxHashMap::default(), None, generics.len())
    }

    /// Returns the generic parameter list of an aggregate declaration.
    pub(super) fn aggregate_generics(&self, definition: DefId) -> Vec<GenericParamDef> {
        self.hir.flattened_generics(definition)
    }

    /// Checks one copied or trait bound against a concrete argument.
    pub(super) fn check_bound(
        &mut self,
        parameter: &GenericParamDef,
        argument: TyId,
        bound: &Bound,
        substs: SubstsId,
        span: Span,
    ) -> bool {
        let name = self.interner.resolve(parameter.name).to_owned();
        match bound {
            Bound::Copy => {
                if self.is_copy_with(argument, &self.env) {
                    true
                } else {
                    self.error(
                        codes::E3043,
                        span,
                        format!(
                            "the type argument `{}` does not satisfy the `Copy` bound of `{name}`",
                            self.ty_text(argument)
                        ),
                    )
                    .help("pass a type that can be copied bitwise, or remove the `Copy` bound")
                    .emit();
                    false
                }
            }
            Bound::Trait(trait_ref) => {
                let goal = self.substitute_trait_ref(*trait_ref, substs);
                match self.prove_trait(goal) {
                    Proof::Proven => true,
                    Proof::Missing => {
                        self.error(
                            codes::E3039,
                            span,
                            format!(
                                "the trait bound `{}` is not satisfied by `{}`",
                                self.display_bound(parameter, *bound),
                                self.ty_text(argument)
                            ),
                        )
                        .help("implement the trait for this type, or change the type argument")
                        .emit();
                        false
                    }
                    Proof::Ambiguous => {
                        self.error(
                            codes::E3040,
                            span,
                            format!(
                                "the trait bound `{}` is ambiguous for `{}`",
                                self.display_bound(parameter, *bound),
                                self.ty_text(argument)
                            ),
                        )
                        .help("remove one of the overlapping implementations")
                        .emit();
                        false
                    }
                    Proof::Recursive => {
                        self.error(
                            codes::E3042,
                            span,
                            format!(
                                "proving `{}` for `{}` exceeded the recursion limit",
                                self.display_bound(parameter, *bound),
                                self.ty_text(argument)
                            ),
                        )
                        .help("break the cycle in the conditional implementations")
                        .emit();
                        false
                    }
                }
            }
        }
    }

    fn substitute_trait_ref(&self, trait_ref: TraitRef, substs: SubstsId) -> TraitRef {
        let self_ty = self.hir.ty_ctxt().substitute(trait_ref.self_ty, substs);
        let args: Vec<TyId> = self
            .hir
            .ty_ctxt()
            .substs_args(trait_ref.args)
            .into_iter()
            .map(|argument| self.hir.ty_ctxt().substitute(argument, substs))
            .collect();
        TraitRef::new(
            trait_ref.trait_def,
            self_ty,
            self.hir.ty_ctxt().substs(&args),
        )
    }

    fn display_bound(&self, parameter: &GenericParamDef, bound: Bound) -> String {
        let name = self.interner.resolve(parameter.name);
        match bound {
            Bound::Copy => format!("{name}: Copy"),
            Bound::Trait(trait_ref) => {
                let trait_name = self.hir.definitions().name(trait_ref.trait_def);
                let trait_name = self.interner.resolve(trait_name);
                let args = self.hir.ty_ctxt().substs_args(trait_ref.args);
                if args.is_empty() {
                    format!("{name}: {trait_name}")
                } else {
                    let args = args
                        .iter()
                        .map(|argument| self.ty_text(*argument))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{name}: {trait_name}<{args}>")
                }
            }
        }
    }

    fn report_unresolved(&self, parameters: &[&GenericParamDef], span: Span) {
        let names = parameters
            .iter()
            .map(|parameter| format!("`{}`", self.interner.resolve(parameter.name)))
            .collect::<Vec<_>>()
            .join(", ");
        let noun = if parameters.len() == 1 {
            "parameter"
        } else {
            "parameters"
        };
        self.error(
            codes::E3038,
            span,
            format!("cannot infer type {noun} {names}"),
        )
        .help("annotate the value, pass a type argument, or supply an argument that determines it")
        .emit();
    }

    /// Returns the impl definition that owns `function`, when it is a method.
    pub(super) fn impl_of_function(&self, function: DefId) -> Option<DefId> {
        self.hir
            .generics(function)
            .and_then(|generics| generics.parent)
            .filter(|parent| self.hir.definitions().get(*parent).kind == DefKind::Impl)
    }

    /// Returns the self type declared by an impl block.
    pub(super) fn impl_self_ty(&self, impl_def: DefId) -> Option<TyId> {
        self.hir
            .inherent_impls()
            .iter()
            .find(|implementation| implementation.def == impl_def)
            .map(|implementation| implementation.self_ty)
            .or_else(|| {
                self.hir
                    .trait_impls()
                    .into_iter()
                    .find(|implementation| implementation.def == impl_def)
                    .map(|implementation| implementation.trait_ref.self_ty)
            })
    }

    /// Derives impl arguments from `Type::<args>::function` syntax.
    ///
    /// The explicit list belongs to the aggregate in the type namespace, not to
    /// the function. Matching the impl self type maps those aggregate arguments
    /// back to the impl's own parameters.
    pub(super) fn associated_seed(
        &mut self,
        function: DefId,
        explicit: Option<&[TyId]>,
        expected: Option<TyId>,
        span: Span,
    ) -> Option<FxHashMap<ParamTyId, TyId>> {
        let Some(impl_def) = self.impl_of_function(function) else {
            return Some(FxHashMap::default());
        };
        let Some(self_ty) = self.impl_self_ty(impl_def) else {
            return Some(FxHashMap::default());
        };
        let TyKind::Adt { adt, .. } = self.hir.ty_ctxt().kind(self_ty) else {
            self.error(
                codes::E3045,
                span,
                "this associated function is not reachable through an aggregate type",
            )
            .emit();
            return None;
        };
        let actual = if let Some(explicit) = explicit {
            Some(
                self.hir
                    .ty_ctxt()
                    .adt_ty_with(adt, self.hir.ty_ctxt().substs(explicit)),
            )
        } else if let Some(expected) = expected
            && let TyKind::Adt {
                adt: expected_adt, ..
            } = self.hir.ty_ctxt().kind(expected)
            && expected_adt == adt
        {
            Some(expected)
        } else {
            None
        };
        let Some(actual) = actual else {
            return Some(FxHashMap::default());
        };
        let mut mapping = FxHashMap::default();
        if super::trait_solver::match_type(self.hir, self_ty, actual, &mut mapping) {
            Some(mapping)
        } else if explicit.is_some() {
            let name = self.def_name(function);
            self.error(
                codes::E3045,
                span,
                format!("no impl of `{}` matches the supplied type arguments", name),
            )
            .help("use type arguments that match the impl self type")
            .emit();
            None
        } else {
            Some(FxHashMap::default())
        }
    }
}
