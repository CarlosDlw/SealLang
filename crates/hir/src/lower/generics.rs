//! Generic parameter registration, scopes, and bound lowering.

use seal_ast::{BoundKind, GenericArgs, GenericParams, PrimitiveType, TraitPath, WhereClause};
use seal_def::{DefData, DefId, DefKind};
use seal_error_codes as codes;
use seal_hash::{FxHashMap, FxHashSet};
use seal_span::Span;
use seal_symbol::Symbol;
use seal_ty::{Bound, GenericParamDef, Generics, ParamEnv, ParamTyId, TraitRef, TyId, TyKind};

use super::Lowering;

impl Lowering<'_> {
    /// Registers one owner's parameters before any of its types are lowered.
    ///
    /// Bounds are queued separately: a bound may name a type declared later in
    /// the file, so it cannot be lowered while the definition is collected.
    pub(super) fn register_generics(
        &mut self,
        owner: DefId,
        params: &GenericParams,
        parent: Option<DefId>,
        implicit_self: bool,
    ) {
        if self.hir.generics(owner).is_some() {
            return;
        }
        let flattened_parent = parent
            .map(|parent| self.hir.flattened_generics(parent))
            .unwrap_or_default();
        let base = flattened_parent.len() as u32;
        let mut names: FxHashSet<Symbol> = flattened_parent
            .iter()
            .map(|parameter| parameter.name)
            .collect();
        let mut declared = Vec::new();

        if implicit_self {
            let name = self.self_symbol;
            let span = self.hir.definitions().get(owner).name_span;
            names.insert(name);
            declared.push(self.define_generic_param(owner, name, span, base).as_self());
        }

        for declaration_id in &params.params {
            let declaration = self.ast.generic_param(*declaration_id);
            let name = declaration.name;
            let name_span = declaration.name_span;
            if !names.insert(name) {
                let first = self
                    .generic_param_names
                    .get(&name)
                    .copied()
                    .unwrap_or(name_span);
                self.error(
                    codes::E2051,
                    name_span,
                    format!(
                        "the generic parameter `{}` is declared twice in this scope",
                        self.interner.resolve(name)
                    ),
                )
                .secondary(first, "first declared here")
                .help("rename one of the parameters")
                .emit();
            }
            if PrimitiveType::from_name(self.interner.resolve(name)).is_some() {
                self.error(
                    codes::E2061,
                    name_span,
                    format!(
                        "the generic parameter `{}` shadows a built-in type",
                        self.interner.resolve(name)
                    ),
                )
                .help("rename the parameter")
                .emit();
            }
            self.generic_param_names.entry(name).or_insert(name_span);
            let index = base + declared.len() as u32;
            declared.push(self.define_generic_param(owner, name, name_span, index));
        }

        self.hir
            .ty_ctxt_mut()
            .insert_generics(Generics::new(owner, parent, declared));
    }

    fn define_generic_param(
        &mut self,
        owner: DefId,
        name: Symbol,
        name_span: Span,
        index: u32,
    ) -> GenericParamDef {
        let span = self.hir.definitions().get(owner).span;
        let definition = self.define(DefData::new(
            DefKind::GenericParam,
            name,
            name_span,
            span,
            Some(owner),
            false,
        ));
        let param = self
            .hir
            .ty_ctxt_mut()
            .push_param(definition, owner, index, name);
        GenericParamDef::new(definition, param, name, index)
    }

    /// Queues bounds until every type declaration and impl header is known.
    pub(super) fn queue_bounds(
        &mut self,
        owner: DefId,
        params: &GenericParams,
        supertraits: &[TraitPath],
        where_clause: &WhereClause,
        self_ty: Option<TyId>,
    ) {
        self.pending_bounds.push(super::PendingBounds {
            owner,
            file: self.current_file,
            params: params.clone(),
            supertraits: supertraits.to_vec(),
            where_clause: where_clause.clone(),
            self_ty,
        });
    }

    /// Lowers inline and where-clause bounds for every owner.
    pub(super) fn lower_pending_bounds(&mut self) {
        let pending = std::mem::take(&mut self.pending_bounds);
        for bounds in pending {
            self.lower_owner_bounds(&bounds);
        }
    }

    fn lower_owner_bounds(&mut self, pending: &super::PendingBounds) {
        let owner = pending.owner;
        let previous_file = self.current_file;
        self.enter_file(pending.file);
        let levels = self.push_generic_scope(owner);
        let saved = self.self_ty;

        let Some(mut generics) = self.hir.generics(owner) else {
            self.pop_generic_scope(levels);
            self.enter_file(previous_file);
            return;
        };
        if let Some(self_ty) = pending.self_ty {
            self.self_ty = Some(self_ty);
        }

        let offset = usize::from(generics.params.first().is_some_and(|param| param.is_self));
        for (position, declaration_id) in pending.params.params.iter().enumerate() {
            let bounds = self.ast.generic_param(*declaration_id).bounds.clone();
            let parameter = &generics.params[offset + position];
            let target = self.hir.ty_ctxt().param_ty(parameter.param);
            let lowered = self.lower_bound_ids(target, &bounds);
            merge_bounds(&mut generics.params[offset + position].bounds, lowered);
        }

        // Supertraits are bounds on the implicit `Self` parameter, so they go
        // through the same environment as inline and where-clause bounds.
        if let Some(position) = generics.params.iter().position(|param| param.is_self) {
            let target = self.hir.ty_ctxt().param_ty(generics.params[position].param);
            let mut lowered = Vec::with_capacity(pending.supertraits.len());
            for path in &pending.supertraits {
                if let Some(bound) = self.lower_trait_bound(target, path)
                    && !lowered.contains(&bound)
                {
                    lowered.push(bound);
                }
            }
            merge_bounds(&mut generics.params[position].bounds, lowered);
        }

        let mut extra: FxHashMap<ParamTyId, Vec<Bound>> = FxHashMap::default();
        let mut env = ParamEnv::new();
        for predicate in &pending.where_clause.predicates {
            let target = self.lower_type(predicate.ty);
            let TyKind::Param(param) = self.hir.ty_ctxt().kind(target) else {
                if !matches!(self.hir.ty_ctxt().kind(target), TyKind::Error) {
                    self.error(
                        codes::E2062,
                        predicate.span,
                        "a where clause may only constrain a type parameter",
                    )
                    .help("move the bound to a type parameter, or remove the predicate")
                    .emit();
                    let _ = self.lower_bound_ids(target, &predicate.bounds);
                }
                continue;
            };
            let lowered = self.lower_bound_ids(target, &predicate.bounds);
            merge_bounds(extra.entry(param).or_default(), lowered);
        }

        for parameter in &mut generics.params {
            if let Some(bounds) = extra.remove(&parameter.param) {
                merge_bounds(&mut parameter.bounds, bounds);
            }
        }
        if !extra.is_empty() {
            for parameter in &generics.params {
                if let Some(bounds) = extra.remove(&parameter.param) {
                    env.insert(parameter.param, bounds);
                }
            }
        }
        self.hir.ty_ctxt_mut().insert_generics(generics);

        for parameter in self.hir.ty_ctxt().flattened_generics(owner) {
            let mut merged = Vec::new();
            merge_bounds(&mut merged, parameter.bounds.clone());
            if let Some(bounds) = extra.remove(&parameter.param) {
                merge_bounds(&mut merged, bounds);
            }
            let mut local = Vec::new();
            merge_bounds(&mut local, env.bounds(parameter.param).to_vec());
            merge_bounds(&mut merged, local);
            if !merged.is_empty() {
                env.insert(parameter.param, merged);
            }
        }
        self.hir.ty_ctxt_mut().set_param_env(owner, env);

        self.self_ty = saved;
        self.pop_generic_scope(levels);
        self.enter_file(previous_file);
    }

    fn lower_bound_ids(&mut self, target: TyId, ids: &[seal_ast::BoundId]) -> Vec<Bound> {
        let mut bounds = Vec::new();
        let mut seen = FxHashSet::default();
        for &id in ids {
            if let Some(bound) = self.lower_bound(target, id) {
                if seen.insert(bound) {
                    bounds.push(bound);
                }
            }
        }
        bounds
    }

    fn lower_bound(&mut self, target: TyId, id: seal_ast::BoundId) -> Option<Bound> {
        match self.ast.bound(id).kind.clone() {
            BoundKind::Copy => Some(Bound::Copy),
            BoundKind::Trait(path) => self.lower_trait_bound(target, &path),
        }
    }

    fn lower_trait_bound(&mut self, target: TyId, path: &TraitPath) -> Option<Bound> {
        let (trait_def, args) = self.lower_bound_trait_path(path)?;
        if self.hir.is_drop_trait(trait_def) {
            let name = self.interner.resolve(path.name);
            self.error(
                codes::E3064,
                path.name_span,
                format!("`{name}` cannot be used as a trait bound"),
            )
            .help(
                "implement `Drop` for the concrete type instead; the destructor runs automatically when a value goes out of scope",
            )
            .emit();
            return None;
        }
        Some(Bound::Trait(TraitRef::new(trait_def, target, args)))
    }

    pub(super) fn lower_bound_trait_path(
        &mut self,
        path: &TraitPath,
    ) -> Option<(DefId, seal_ty::SubstsId)> {
        let Some(trait_def) = self.trait_def(path.name) else {
            let name = self.interner.resolve(path.name);
            if self.type_exists(path.name) || self.value_exists(path.name) {
                self.error(
                    codes::E2060,
                    path.name_span,
                    format!("`{name}` is not a trait"),
                )
                .help("declare the requirement as a trait, then implement it")
                .emit();
            } else {
                self.error(
                    codes::E2057,
                    path.name_span,
                    format!("cannot find trait `{name}` in this scope"),
                )
                .help("declare the trait, or check its spelling")
                .emit();
            }
            if let Some(args) = &path.args {
                for argument in &args.args {
                    let _ = self.lower_type_inner(*argument, true, false);
                }
            }
            return None;
        };
        self.warn_deprecated(trait_def, path.name_span);
        if let Some(kind) = self.hir.fn_trait_kind(trait_def) {
            return self
                .lower_fn_trait_arguments(kind, path)
                .map(|args| (trait_def, args));
        }
        let expected = self
            .hir
            .generics(trait_def)
            .map(|generics| generics.params.len() - 1)
            .unwrap_or(0);
        let args = self.lower_type_arguments(path.args.as_ref(), expected, true, path.name_span)?;
        Some((trait_def, args))
    }

    /// Lowers and validates the two arguments of a function trait.
    ///
    /// The sugar always supplies a tuple of parameter types and a result type.
    /// The explicit spelling has to follow the same shape: the first argument
    /// must be a tuple (or `()`), and the result must be `Sized`.
    fn lower_fn_trait_arguments(
        &mut self,
        kind: seal_ty::ClosureKind,
        path: &TraitPath,
    ) -> Option<seal_ty::SubstsId> {
        let name = kind.as_str();
        let Some(args) = &path.args else {
            self.error(
                codes::E3096,
                path.name_span,
                format!("the function trait `{name}` needs argument and result types"),
            )
            .help(format!("write the bound as `{name}(A, B) -> R`"))
            .emit();
            return None;
        };
        if args.args.len() != 2 {
            self.error(
                codes::E3096,
                args.span,
                format!(
                    "the function trait `{name}` takes two type arguments, a tuple of parameters and a result, but {} were supplied",
                    args.args.len()
                ),
            )
            .help(format!("write the bound as `{name}(A, B) -> R`"))
            .emit();
            for argument in &args.args {
                let _ = self.lower_type_inner(*argument, true, false);
            }
            return None;
        }
        let lowered: Vec<TyId> = args
            .args
            .iter()
            .map(|argument| self.lower_type_inner(*argument, true, false))
            .collect();
        let parameters = lowered[0];
        if !matches!(
            self.hir.ty_ctxt().kind(parameters),
            TyKind::Tuple(_) | TyKind::Unit
        ) {
            let display = self.display_ty(parameters);
            self.error(
                codes::E3096,
                args.span,
                format!(
                    "the first argument of `{name}` must be a tuple of parameter types, found `{display}`"
                ),
            )
            .help(format!("write the bound as `{name}(A, B) -> R`, or remove the bound"))
            .emit();
            return None;
        }
        let output = lowered[1];
        if !self.hir.ty_ctxt().is_sized(output)
            && !matches!(self.hir.ty_ctxt().kind(output), TyKind::Error)
        {
            let display = self.display_ty(output);
            self.error(
                codes::E3096,
                args.span,
                format!("the result `{display}` of `{name}` must be sized"),
            )
            .help("return the value behind a pointer, or use a sized result type")
            .emit();
            return None;
        }
        Some(self.hir.ty_ctxt().substs(&lowered))
    }

    /// Builds the semantic declaration of every trait from its lowered bounds
    /// and method signatures.
    ///
    /// Trait signatures keep `Self` as the trait's first parameter. A concrete
    /// type is substituted only when an implementation is validated.
    pub(super) fn build_trait_definitions(&mut self) {
        let traits = self.trait_items.clone();
        for (item, trait_def) in traits {
            self.enter_file(item.file);
            let Some(generics) = self.hir.generics(trait_def) else {
                continue;
            };
            let Some(order) = self.trait_method_order.get(&trait_def).cloned() else {
                continue;
            };
            let name = self.hir.definitions().name(trait_def);
            let mut methods = Vec::new();
            for method_name in order {
                let Some(raw) = self.trait_methods.get(&(trait_def, method_name)).cloned() else {
                    continue;
                };
                let levels = self.push_generic_scope(raw.def);
                let params = raw
                    .params
                    .iter()
                    .map(|(ty, _)| self.lower_type(*ty))
                    .collect();
                let ret = match raw.ret {
                    Some(ty) => self.lower_type(ty),
                    None => self.hir.ty_ctxt().unit_ty(),
                };
                self.pop_generic_scope(levels);
                let method_generics = self
                    .hir
                    .generics(raw.def)
                    .unwrap_or_else(|| Generics::new(raw.def, Some(trait_def), Vec::new()));
                let receiver = raw.receiver.map(|receiver| seal_ty::TraitMethodReceiver {
                    reference: match receiver {
                        crate::ReceiverKind::Value => None,
                        crate::ReceiverKind::Shared => Some(seal_ty::RefKind::Shared),
                        crate::ReceiverKind::Exclusive => Some(seal_ty::RefKind::Exclusive),
                    },
                });
                methods.push(
                    seal_ty::TraitMethodDef::new(
                        raw.def,
                        raw.name,
                        seal_ty::GenericSignature::new(params, ret),
                        method_generics,
                        receiver,
                    )
                    .with_body(raw.body.is_some())
                    .with_unsafe(raw.is_unsafe),
                );
            }

            let self_param = generics
                .params
                .iter()
                .find(|parameter| parameter.is_self)
                .map(|parameter| parameter.param);
            let method_defs: Vec<seal_def::DefId> = methods
                .iter()
                .filter(|method| {
                    self.trait_methods
                        .get(&(trait_def, method.name))
                        .is_some_and(|raw| raw.body.is_some())
                })
                .map(|method| method.def)
                .collect();
            let trait_arguments: Vec<seal_ty::TyId> = generics
                .params
                .iter()
                .filter(|parameter| !parameter.is_self)
                .map(|parameter| self.hir.ty_ctxt().param_ty(parameter.param))
                .collect();
            let mut supertraits = Vec::new();
            if let Some(self_param) = self_param {
                for bound in self.hir.param_env(trait_def).bounds(self_param) {
                    if let Bound::Trait(trait_ref) = bound {
                        if !supertraits.contains(trait_ref) {
                            supertraits.push(*trait_ref);
                        }
                    }
                }
            }
            self.hir
                .ty_ctxt_mut()
                .insert_trait_def(seal_ty::TraitDef::new(
                    trait_def,
                    name,
                    generics,
                    supertraits,
                    methods,
                ));

            // A default body may call any method of its own trait through
            // `self`. The implicit `Self: Trait` bound is visible while
            // checking that body, but it is not a declared bound: keeping it
            // out of `Generics` leaves coherence, object safety, and the
            // trait's own supertrait list unchanged.
            if let Some(self_param) = self_param {
                let self_ty = self.hir.ty_ctxt().param_ty(self_param);
                let trait_ref = seal_ty::TraitRef::new(
                    trait_def,
                    self_ty,
                    self.hir.ty_ctxt().substs(&trait_arguments),
                );
                for method in method_defs {
                    let mut env = self.hir.param_env(method);
                    let already = env.bounds(self_param).iter().any(|bound| {
                        matches!(bound, Bound::Trait(existing) if existing.trait_def == trait_def && existing.self_ty == self_ty)
                    });
                    if !already {
                        env.insert(self_param, vec![Bound::Trait(trait_ref)]);
                        self.hir.ty_ctxt_mut().set_param_env(method, env);
                    }
                }
            }
        }
    }

    /// Enters the generic scopes of `owner` and every parent.
    ///
    /// Returns the number of ribs pushed so the caller can pop the same count.
    pub(super) fn push_generic_scope(&mut self, owner: DefId) -> usize {
        let mut chain = Vec::new();
        let mut current = Some(owner);
        while let Some(def) = current {
            let Some(generics) = self.hir.generics(def) else {
                break;
            };
            current = generics.parent;
            chain.push(generics);
        }
        chain.reverse();
        let count = chain.len();
        for generics in chain {
            self.generic_scopes.push(&generics.params);
        }
        count
    }

    pub(super) fn pop_generic_scope(&mut self, levels: usize) {
        for _ in 0..levels {
            self.generic_scopes.pop();
        }
    }

    pub(super) fn generic_scope_param(&self, name: Symbol) -> Option<ParamTyId> {
        self.generic_scopes.lookup(name)
    }

    pub(super) fn generic_parameter_declared_somewhere(&self, name: Symbol) -> Option<Span> {
        self.generic_param_names.get(&name).copied()
    }

    /// Lowers an explicit type-argument list.
    ///
    /// Returns `None` when the list is omitted or malformed; type annotations
    /// require a complete list because there is no expression to infer from.
    pub(super) fn lower_type_arguments(
        &mut self,
        args: Option<&GenericArgs>,
        expected: usize,
        unknown_parameter: bool,
        span: Span,
    ) -> Option<seal_ty::SubstsId> {
        let Some(args) = args else {
            if expected == 0 {
                return Some(self.hir.ty_ctxt().empty_substs());
            }
            self.error(
                codes::E2052,
                span,
                format!("expected {expected} type argument(s), but none were supplied"),
            )
            .help("supply exactly one argument per declared parameter")
            .emit();
            return None;
        };
        if args.args.len() != expected {
            self.report_argument_count(args, expected);
            for argument in &args.args {
                let _ = self.lower_type_inner(*argument, unknown_parameter, false);
            }
            return None;
        }
        let lowered: Vec<TyId> = args
            .args
            .iter()
            .map(|argument| self.lower_type_inner(*argument, unknown_parameter, false))
            .collect();
        Some(self.hir.ty_ctxt().substs(&lowered))
    }

    /// Lowers an expression's explicit type-argument list.
    ///
    /// Omitted arguments remain `None` so type checking can infer them.
    pub(super) fn lower_expr_arguments(
        &mut self,
        args: Option<&GenericArgs>,
        expected: usize,
    ) -> Option<seal_ty::SubstsId> {
        let args = args?;
        let valid = args.args.len() == expected;
        if !valid {
            self.report_argument_count(args, expected);
        }
        let lowered: Vec<TyId> = args
            .args
            .iter()
            .map(|argument| self.lower_type_inner(*argument, false, false))
            .collect();
        valid.then(|| self.hir.ty_ctxt().substs(&lowered))
    }

    fn report_argument_count(&mut self, args: &GenericArgs, expected: usize) {
        if expected == 0 {
            self.error(
                codes::E2053,
                args.span,
                format!(
                    "this declaration is not generic, but {} type argument(s) were supplied",
                    args.args.len()
                ),
            )
            .help("remove the type argument list")
            .emit();
        } else {
            self.error(
                codes::E2052,
                args.span,
                format!(
                    "expected {expected} type argument(s), but {} were supplied",
                    args.args.len()
                ),
            )
            .help("supply exactly one argument per declared parameter")
            .emit();
        }
    }
}

fn merge_bounds(existing: &mut Vec<Bound>, additional: Vec<Bound>) {
    for bound in additional {
        if !existing.contains(&bound) {
            existing.push(bound);
        }
    }
}
