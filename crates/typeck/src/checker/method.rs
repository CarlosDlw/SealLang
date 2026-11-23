//! Intrinsic method resolution.
//!
//! Lookup is deliberately explicit: the receiver's reference layers are peeled
//! until a scalar, array, or slice reference is found, then the catalog chooses
//! one entry. The resolved call records every dereference and the way the
//! receiver reaches MIR, so later phases never repeat inference.

use seal_def::DefId;
use seal_error_codes as codes;
use seal_hash::{FxHashMap, FxHashSet};
use seal_hir::{HirExprId, ReceiverKind};
use seal_methods::{
    IntrinsicOp, MethodDef, MethodId, MethodOwner, ParamKind, ReceiverMode, ReturnKind,
};
use seal_span::Span;
use seal_symbol::Symbol;
use seal_ty::{RefKind, TyId, TyKind};

use super::Checker;
use super::PlaceMode;
use super::dynamic::DynamicCall;
use super::flow::Flow;
use super::trait_solver::match_type;
use crate::results::{MethodKind, ReceiverAdjust, ReceiverForm, ResolvedMethod, ResolvedPlace};

const MAX_RECEIVER_DEREFS: usize = 16;

/// The checked pieces of one `receiver.name(arguments)` expression.
pub(super) struct MethodCall<'a> {
    pub(super) expr: HirExprId,
    pub(super) receiver: HirExprId,
    pub(super) name: Symbol,
    pub(super) name_span: Span,
    pub(super) generic_args: Option<seal_ty::SubstsId>,
    pub(super) args: &'a [HirExprId],
    pub(super) span: Span,
}

impl Checker<'_> {
    pub(super) fn check_method_call(
        &mut self,
        call: MethodCall<'_>,
        expected: Option<TyId>,
        flow: &mut Flow,
    ) -> TyId {
        let MethodCall {
            expr,
            receiver,
            name,
            name_span,
            generic_args,
            args,
            span,
        } = call;
        // A receiver is a place: the resolved receiver decides whether it is
        // moved or borrowed, so typing it must not read a non-`Copy` pointee.
        let mut receiver_ty = self.infer_expr_mode(receiver, None, flow, PlaceMode::Borrow);
        let mut receiver_place = self.results.place(receiver).cloned();

        let callable_ty = self.resolve_deep(receiver_ty);
        let is_closure = self.closure_of_ty(callable_ty).is_some();
        let has_fn_bound = !self.fn_trait_bounds_of_ty(callable_ty).is_empty();
        if (is_closure || has_fn_bound) && self.is_function_trait_method(name) {
            for arg in args {
                self.infer_expr(*arg, None, flow);
            }
            let name = self.interner.resolve(name).to_owned();
            let subject = if is_closure {
                "a closure"
            } else {
                "a value with a function-trait bound"
            };
            self.error(
                codes::E3096,
                name_span,
                format!("{subject} is not called through the method `{name}`"),
            )
            .help("write `f(...)` instead of `f.call(...)`")
            .emit();
            return self.error_ty();
        }

        if let Some(ty) = self.check_dynamic_method_call(
            DynamicCall {
                expr,
                receiver,
                receiver_ty,
                name,
                name_span,
                generic_args,
                args,
                span,
                expected,
            },
            flow,
        ) {
            return ty;
        }

        let target = match self.user_method_target(receiver_ty, name, name_span) {
            UserLookup::NotAStruct => None,
            UserLookup::Rejected => {
                for arg in args {
                    self.infer_expr(*arg, None, flow);
                }
                return self.error_ty();
            }
            UserLookup::Method(target) => Some(target),
        };
        if let Some(target) = target {
            if user_method_receiver(&target) == Some(ReceiverKind::Exclusive)
                && receiver_place.as_ref().is_some_and(needs_exclusive_reindex)
            {
                receiver_ty =
                    self.infer_expr_mode(receiver, None, flow, PlaceMode::BorrowExclusive);
                receiver_place = self.results.place(receiver).cloned();
            }
            return self.check_user_method(
                UserCall {
                    expr,
                    receiver,
                    receiver_ty,
                    receiver_place,
                    target,
                    name,
                    name_span,
                    generic_args,
                    args,
                    span,
                    expected,
                },
                flow,
            );
        }

        let resolution = self.resolve_method(
            receiver,
            receiver_ty,
            receiver_place.as_ref(),
            name,
            name_span,
            receiver_ty,
        );
        let Some(resolution) = resolution else {
            for arg in args {
                self.infer_expr(*arg, None, flow);
            }
            return self.error_ty();
        };

        if resolution.def.receiver == ReceiverMode::Exclusive
            && receiver_place.as_ref().is_some_and(needs_exclusive_reindex)
        {
            let _ = self.infer_expr_mode(receiver, None, flow, PlaceMode::BorrowExclusive);
            receiver_place = self.results.place(receiver).cloned();
        }

        if resolution.def.receiver == ReceiverMode::Value
            && receiver_place
                .as_ref()
                .is_some_and(|place| place.contains_deref())
            && !self.is_copy(resolution.receiver_ty)
        {
            self.report_move_out_of_reference(resolution.receiver_ty, self.hir.expr(receiver).span);
        }

        if resolution.def.receiver == ReceiverMode::Exclusive
            && !self.receiver_is_writable(receiver, receiver_place.as_ref(), &resolution)
        {
            self.error(
                codes::E3029,
                self.hir.expr(receiver).span,
                format!(
                    "method `{}` needs to modify its receiver",
                    resolution.def.name
                ),
            )
            .help("borrow the receiver exclusively or declare the binding with `let mut`")
            .emit();
        }

        if resolution.def.requires_copy_element && !self.is_copy(resolution.element_ty) {
            self.error(
                codes::E3030,
                span,
                format!(
                    "method `fill` needs a `Copy` element, but `{}` is not `Copy`",
                    self.ty_text(resolution.element_ty)
                ),
            )
            .help("copy the value into each element explicitly, or use a `Copy` element type")
            .emit();
        }

        if resolution.def.params.len() != args.len() {
            self.error(
                codes::E3028,
                span,
                format!(
                    "method `{}` takes {} argument(s) but {} were supplied",
                    resolution.def.name,
                    resolution.def.params.len(),
                    args.len()
                ),
            )
            .help("pass exactly one argument for each parameter")
            .emit();
        }

        let mut arg_tys = Vec::with_capacity(args.len());
        for (index, arg) in args.iter().enumerate() {
            match resolution.def.params.get(index) {
                Some(kind) => {
                    let expected = self.param_ty(*kind, &resolution);
                    self.check_expr(*arg, expected, flow);
                    arg_tys.push(expected);
                }
                None => {
                    self.infer_expr(*arg, None, flow);
                }
            }
        }

        let ret_ty = self.return_ty(&resolution);
        if !self.is_error(ret_ty) {
            self.results.record_method(
                expr,
                ResolvedMethod {
                    kind: MethodKind::Intrinsic(resolution.id),
                    receiver_adjust: ReceiverAdjust::None,
                    receiver_ty: resolution.receiver_ty,
                    receiver_form: resolution.form,
                    receiver_derefs: resolution.derefs,
                    array_to_slice: resolution.array_to_slice,
                    arg_tys,
                    ret_ty,
                },
            );
        }
        ret_ty
    }

    /// Looks up a user method on a nominal receiver before intrinsic lookup.
    ///
    /// An inherent candidate wins; otherwise matching trait impls are searched.
    /// A type parameter searches only its visible bounds. The receiver is not a
    /// nominal type in [`UserLookup::NotAStruct`], so the caller falls through to
    /// the intrinsic catalog.
    fn user_method_target(
        &mut self,
        receiver_ty: TyId,
        name: Symbol,
        name_span: Span,
    ) -> UserLookup {
        let mut base = receiver_ty;
        while let TyKind::Ref { pointee, .. } = self.hir.ty_ctxt().kind(base) {
            base = pointee;
        }
        match self.hir.ty_ctxt().kind(base) {
            TyKind::Adt { .. } => self.adimpl_method_target(base, receiver_ty, name, name_span),
            TyKind::Param(param) => {
                self.bound_method_target(base, receiver_ty, param, name, name_span)
            }
            TyKind::Str => self.str_method_target(base, receiver_ty, name, name_span),
            TyKind::Error => UserLookup::Rejected,
            _ => self.primitive_method_target(base, receiver_ty, name, name_span),
        }
    }

    /// Looks up an inherent `impl str` method before trait or catalog lookup.
    ///
    /// A registered method wins over a trait method and over a catalog entry
    /// with the same name, matching nominal inherent lookup. When the name is
    /// not registered, the trait and catalog paths remain available.
    fn str_method_target(
        &mut self,
        base: TyId,
        receiver_ty: TyId,
        name: Symbol,
        name_span: Span,
    ) -> UserLookup {
        let count = self.hir.str_methods(name).len();
        if count == 0 {
            return self.primitive_method_target(base, receiver_ty, name, name_span);
        }
        if count > 1 {
            self.error(
                codes::E3045,
                name_span,
                format!(
                    "more than one inherent impl declares `{}` for `{}`",
                    self.interner.resolve(name),
                    self.ty_text(base)
                ),
            )
            .help("remove one of the overlapping inherent methods")
            .emit();
            return UserLookup::Rejected;
        }
        let function = self.hir.str_methods(name)[0];
        let Some(receiver) = self.hir.receiver(function) else {
            self.report_associated_as_method(base, name, name_span);
            return UserLookup::Rejected;
        };
        UserLookup::Method(UserMethod::Concrete {
            function,
            receiver,
            source: MethodSource::Inherent,
            self_ty: base,
            seed: FxHashMap::default(),
        })
    }

    /// Looks up a trait method implemented for a primitive, tuple, or reference.
    ///
    /// Implemented traits take precedence over the intrinsic catalog, so a
    /// user trait can give a primitive a method with an intrinsic name. When no
    /// impl declares the name, the caller falls through to the catalog.
    fn primitive_method_target(
        &mut self,
        base: TyId,
        receiver_ty: TyId,
        name: Symbol,
        name_span: Span,
    ) -> UserLookup {
        let candidates = self.trait_impl_candidates(receiver_ty, base, name);
        match candidates.len() {
            0 => UserLookup::NotAStruct,
            1 => {
                let candidate = candidates.into_iter().next().unwrap();
                UserLookup::Method(candidate.into_user_method())
            }
            _ => {
                let traits: Vec<DefId> = candidates
                    .iter()
                    .map(|candidate| candidate.trait_def)
                    .collect();
                self.report_ambiguous_trait_method(base, name, name_span, &traits);
                UserLookup::Rejected
            }
        }
    }

    /// Returns every trait impl whose self type matches the receiver.
    ///
    /// The unpeeled receiver is tried first, so an impl for a reference type is
    /// reachable; the peeled base is the automatic-deref fallback.
    fn trait_impl_candidates(
        &self,
        receiver_ty: TyId,
        base: TyId,
        name: Symbol,
    ) -> Vec<TraitCandidate> {
        let mut implementations = self.hir.ty_ctxt().trait_impls_matching(receiver_ty);
        if receiver_ty != base {
            let mut seen: FxHashSet<DefId> = implementations
                .iter()
                .map(|implementation| implementation.def)
                .collect();
            for implementation in self.hir.ty_ctxt().trait_impls_matching(base) {
                if seen.insert(implementation.def) {
                    implementations.push(implementation);
                }
            }
        }
        let mut candidates = Vec::new();
        for implementation in implementations {
            let pattern = implementation.trait_ref.self_ty;
            let mut seed = FxHashMap::default();
            let self_ty = if match_type(self.hir, pattern, receiver_ty, &mut seed) {
                receiver_ty
            } else if receiver_ty != base {
                let mut peeled = FxHashMap::default();
                if match_type(self.hir, pattern, base, &mut peeled) {
                    seed = peeled;
                    base
                } else {
                    continue;
                }
            } else {
                continue;
            };

            let kind = if let Some(function) = implementation.method(name) {
                let Some(receiver) = self.hir.receiver(function) else {
                    continue;
                };
                TraitCandidateKind::Provided { function, receiver }
            } else {
                let default = self
                    .hir
                    .trait_definition(implementation.trait_ref.trait_def)
                    .and_then(|definition| definition.method(name).cloned())
                    .filter(|method| self.hir.body_of(method.def).is_some());
                let Some(method) = default else {
                    continue;
                };
                if method.receiver.is_none() {
                    continue;
                }
                TraitCandidateKind::Default {
                    trait_ref: self.instantiate_trait_ref(&implementation, self_ty, &seed),
                    method,
                }
            };
            candidates.push(TraitCandidate {
                trait_def: implementation.trait_ref.trait_def,
                kind,
                seed,
                self_ty,
            });
        }
        candidates
    }

    /// Builds the concrete trait reference for an impl selected by its self
    /// type. A default method has no impl function, so its `[Self, args]`
    /// substitution must come from the matched impl header itself.
    fn instantiate_trait_ref(
        &self,
        implementation: &seal_ty::TraitImplDef,
        self_ty: TyId,
        seed: &FxHashMap<seal_ty::ParamTyId, TyId>,
    ) -> seal_ty::TraitRef {
        let tcx = self.hir.ty_ctxt();
        let impl_arguments: Vec<TyId> = implementation
            .generics
            .params
            .iter()
            .map(|parameter| {
                seed.get(&parameter.param)
                    .copied()
                    .unwrap_or_else(|| tcx.param_ty(parameter.param))
            })
            .collect();
        let substs = tcx.substs(&impl_arguments);
        let trait_arguments: Vec<TyId> = tcx
            .substs_args(implementation.trait_ref.args)
            .iter()
            .map(|argument| tcx.substitute(*argument, substs))
            .collect();
        seal_ty::TraitRef::new(
            implementation.trait_ref.trait_def,
            self_ty,
            tcx.substs(&trait_arguments),
        )
    }

    fn adimpl_method_target(
        &mut self,
        base: TyId,
        receiver_ty: TyId,
        name: Symbol,
        name_span: Span,
    ) -> UserLookup {
        let mut inherent = Vec::new();
        for implementation in self.hir.inherent_impls() {
            let Some(function) = implementation
                .methods
                .iter()
                .find_map(|(candidate, def)| (*candidate == name).then_some(*def))
            else {
                continue;
            };
            let mut seed = FxHashMap::default();
            if match_type(self.hir, implementation.self_ty, base, &mut seed) {
                inherent.push((function, seed));
            }
        }
        if inherent.len() > 1 {
            self.error(
                codes::E3045,
                name_span,
                format!(
                    "more than one inherent impl declares `{}` for `{}`",
                    self.interner.resolve(name),
                    self.ty_text(base)
                ),
            )
            .help("remove one of the overlapping inherent methods")
            .emit();
            return UserLookup::Rejected;
        }
        if let Some((function, seed)) = inherent.pop() {
            return match self.hir.receiver(function) {
                Some(receiver) => UserLookup::Method(UserMethod::Concrete {
                    function,
                    receiver,
                    source: MethodSource::Inherent,
                    self_ty: base,
                    seed,
                }),
                None => {
                    self.report_associated_as_method(base, name, name_span);
                    UserLookup::Rejected
                }
            };
        }

        let candidates = self.trait_impl_candidates(receiver_ty, base, name);
        match candidates.len() {
            0 => {
                self.report_missing_method(receiver_ty, name, name_span);
                UserLookup::Rejected
            }
            1 => {
                let candidate = candidates.into_iter().next().unwrap();
                UserLookup::Method(candidate.into_user_method())
            }
            _ => {
                let traits: Vec<DefId> = candidates
                    .iter()
                    .map(|candidate| candidate.trait_def)
                    .collect();
                self.report_ambiguous_trait_method(base, name, name_span, &traits);
                UserLookup::Rejected
            }
        }
    }

    fn bound_method_target(
        &mut self,
        base: TyId,
        receiver_ty: TyId,
        param: seal_ty::ParamTyId,
        name: Symbol,
        name_span: Span,
    ) -> UserLookup {
        let mut candidates = Vec::new();
        for trait_ref in self.visible_traits(param) {
            let Some(definition) = self.hir.trait_definition(trait_ref.trait_def) else {
                continue;
            };
            let Some(method) = definition.method(name) else {
                continue;
            };
            if method.receiver.is_none() {
                continue;
            }
            candidates.push(UserMethod::Bound {
                trait_ref,
                method: method.clone(),
                self_ty: base,
            });
        }
        match candidates.len() {
            0 => {
                self.report_missing_method(receiver_ty, name, name_span);
                UserLookup::Rejected
            }
            1 => UserLookup::Method(candidates.pop().unwrap()),
            _ => {
                self.error(
                    codes::E3040,
                    name_span,
                    format!(
                        "the method `{}` on `{}` is provided by more than one bound",
                        self.interner.resolve(name),
                        self.ty_text(base)
                    ),
                )
                .help("remove one of the bounds, or add an inherent method")
                .emit();
                UserLookup::Rejected
            }
        }
    }

    /// Returns every trait visible on `param`, including supertraits.
    pub(super) fn visible_traits(&self, param: seal_ty::ParamTyId) -> Vec<seal_ty::TraitRef> {
        let mut traits = Vec::new();
        for bound in self.env.bounds(param) {
            let seal_ty::Bound::Trait(trait_ref) = bound else {
                continue;
            };
            self.collect_visible_traits(*trait_ref, &mut traits);
        }
        traits
    }

    fn collect_visible_traits(
        &self,
        trait_ref: seal_ty::TraitRef,
        traits: &mut Vec<seal_ty::TraitRef>,
    ) {
        if traits.contains(&trait_ref) {
            return;
        }
        traits.push(trait_ref);
        let Some(definition) = self.hir.trait_definition(trait_ref.trait_def) else {
            return;
        };
        for supertrait in &definition.supertraits {
            let Some(instantiated) = self
                .hir
                .ty_ctxt()
                .instantiate_supertrait(*supertrait, trait_ref)
            else {
                continue;
            };
            self.collect_visible_traits(instantiated, traits);
        }
    }

    pub(super) fn report_associated_as_method(&self, base: TyId, name: Symbol, span: Span) {
        self.error(
            codes::E2019,
            span,
            format!(
                "`{}` is an associated function, not a method",
                self.interner.resolve(name)
            ),
        )
        .help(format!(
            "call it as `{}::{}()`, or give it a `self` receiver",
            self.ty_text(base),
            self.interner.resolve(name)
        ))
        .emit();
    }

    pub(super) fn report_ambiguous_trait_method(
        &self,
        base: TyId,
        name: Symbol,
        span: Span,
        traits: &[DefId],
    ) {
        let traits = traits
            .iter()
            .map(|definition| {
                format!(
                    "`{}`",
                    self.interner
                        .resolve(self.hir.definitions().name(*definition))
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        self.error(
            codes::E2028,
            span,
            format!(
                "the method `{}` on `{}` is declared by more than one trait",
                self.interner.resolve(name),
                self.ty_text(base)
            ),
        )
        .help(format!(
            "the traits are {traits}; rename the method in one of them, or add an inherent method"
        ))
        .emit();
    }

    fn check_user_method(&mut self, call: UserCall<'_>, flow: &mut Flow) -> TyId {
        let UserCall {
            expr,
            receiver,
            receiver_ty,
            receiver_place,
            target,
            name,
            name_span,
            generic_args,
            args,
            span,
            expected,
        } = call;
        let drop_call = match &target {
            UserMethod::Concrete { source, .. } => {
                matches!(source, MethodSource::Trait(trait_def) if self.hir.is_drop_trait(*trait_def))
            }
            UserMethod::Bound { method, .. } => method.def == self.hir.drop_method(),
        };
        if drop_call {
            self.report_drop_call(span);
            for arg in args {
                self.infer_expr(*arg, None, flow);
            }
            return self.error_ty();
        }
        match &target {
            UserMethod::Concrete {
                function,
                source: MethodSource::Inherent,
                ..
            } => self.warn_deprecated(*function, name_span),
            UserMethod::Concrete {
                function,
                source: MethodSource::Trait(trait_def),
                ..
            } => {
                self.warn_deprecated(*function, name_span);
                if let Some(method) = self
                    .hir
                    .trait_definition(*trait_def)
                    .and_then(|definition| definition.method(name).map(|method| method.def))
                {
                    self.warn_deprecated(method, name_span);
                }
            }
            UserMethod::Bound { method, .. } => self.warn_deprecated(method.def, name_span),
        }
        // The reachable declaration is the method itself for an inherent
        // impl, and the trait for a trait method: a trait method inherits the
        // trait's visibility and cannot widen it.
        let visibility_def = match &target {
            UserMethod::Concrete {
                function,
                source: MethodSource::Inherent,
                ..
            } => Some(*function),
            UserMethod::Concrete {
                source: MethodSource::Trait(trait_def),
                ..
            } => Some(*trait_def),
            UserMethod::Bound { trait_ref, .. } => Some(trait_ref.trait_def),
        };
        if let Some(def) = visibility_def {
            let _ = self.require_item_visible(def, span);
        }
        let is_unsafe = match &target {
            UserMethod::Concrete { function, .. } => self
                .signature(*function)
                .is_some_and(|signature| signature.is_unsafe),
            UserMethod::Bound { method, .. } => method.is_unsafe,
        };
        self.require_unsafe_call(span, is_unsafe, name);
        match target {
            UserMethod::Concrete {
                function,
                receiver: receiver_kind,
                source,
                self_ty,
                seed,
            } => self.check_concrete_method(
                ConcreteCall {
                    expr,
                    receiver,
                    receiver_ty,
                    receiver_place,
                    function,
                    receiver_kind,
                    source,
                    self_ty,
                    seed,
                    name,
                    generic_args,
                    args,
                    span,
                    expected,
                },
                flow,
            ),
            UserMethod::Bound {
                trait_ref,
                method,
                self_ty,
            } => self.check_bound_method(
                BoundCall {
                    expr,
                    receiver,
                    receiver_ty,
                    receiver_place,
                    trait_ref,
                    method,
                    self_ty,
                    name,
                    generic_args,
                    args,
                    span,
                    expected,
                },
                flow,
            ),
        }
    }

    fn check_concrete_method(&mut self, call: ConcreteCall<'_>, flow: &mut Flow) -> TyId {
        let ConcreteCall {
            expr,
            receiver,
            receiver_ty,
            receiver_place,
            function,
            receiver_kind,
            source,
            self_ty,
            seed,
            name,
            generic_args,
            args,
            span,
            expected,
        } = call;
        let own_count = self
            .hir
            .generics(function)
            .map(|generics| generics.params.len())
            .unwrap_or(0);
        let explicit = self.method_type_arguments(generic_args, own_count, span);
        let instantiated =
            self.instantiate_function(function, &seed, explicit.as_deref(), expected);
        let Some(expected_receiver) = instantiated.params.first().copied() else {
            self.report_missing_method(receiver_ty, name, self.hir.expr(receiver).span);
            return self.error_ty();
        };
        let adjust = self.receiver_adjustment(
            receiver,
            receiver_ty,
            self_ty,
            expected_receiver,
            receiver_kind,
            name,
        );
        self.check_receiver(
            receiver,
            self_ty,
            receiver_place.as_ref(),
            adjust,
            receiver_kind,
            name,
        );
        let expected_args = instantiated.params.len().saturating_sub(1);
        if expected_args != args.len() {
            self.error(
                codes::E3028,
                span,
                format!(
                    "method `{}` takes {} argument(s) but {} were supplied",
                    self.interner.resolve(name),
                    expected_args,
                    args.len()
                ),
            )
            .help("pass exactly one argument for each parameter after the receiver")
            .emit();
        }
        for (index, arg) in args.iter().enumerate() {
            match instantiated.params.get(index + 1) {
                Some(expected) => self.check_expr(*arg, *expected, flow),
                None => {
                    self.infer_expr(*arg, None, flow);
                }
            }
        }
        let Some((params, ret_ty, _)) =
            self.finish_function_instantiation(expr, function, instantiated, span)
        else {
            return self.error_ty();
        };
        if !self.is_error(ret_ty) {
            let kind = match source {
                MethodSource::Inherent => MethodKind::User {
                    function,
                    receiver: receiver_kind,
                },
                MethodSource::Trait(trait_def) => MethodKind::Trait {
                    function,
                    receiver: receiver_kind,
                    trait_def,
                },
            };
            self.results.record_method(
                expr,
                ResolvedMethod {
                    kind,
                    receiver_adjust: adjust,
                    receiver_ty: self_ty,
                    receiver_form: ReceiverForm::Place,
                    receiver_derefs: 0,
                    array_to_slice: false,
                    arg_tys: params.into_iter().skip(1).collect(),
                    ret_ty,
                },
            );
        }
        ret_ty
    }

    fn check_bound_method(&mut self, call: BoundCall<'_>, flow: &mut Flow) -> TyId {
        let BoundCall {
            expr,
            receiver,
            receiver_ty,
            receiver_place,
            trait_ref,
            method,
            self_ty,
            name,
            generic_args,
            args,
            span,
            expected,
        } = call;
        let receiver_kind =
            trait_receiver_kind(method.receiver.expect("bound methods require a receiver"));
        let own_count = method.generics.params.len();
        let explicit = self.method_type_arguments(generic_args, own_count, span);
        let Some(instantiated) =
            self.instantiate_trait_method(trait_ref, &method, explicit.as_deref(), expected)
        else {
            return self.error_ty();
        };
        let expected_receiver = match receiver_kind {
            ReceiverKind::Value => self_ty,
            ReceiverKind::Shared => self.ref_ty(RefKind::Shared, self_ty),
            ReceiverKind::Exclusive => self.ref_ty(RefKind::Exclusive, self_ty),
        };
        let adjust = self.receiver_adjustment(
            receiver,
            receiver_ty,
            self_ty,
            expected_receiver,
            receiver_kind,
            name,
        );
        self.check_receiver(
            receiver,
            self_ty,
            receiver_place.as_ref(),
            adjust,
            receiver_kind,
            name,
        );
        let Some((params, ret_ty, _)) =
            self.finish_trait_method_call(expr, instantiated, &method, args, flow, span)
        else {
            return self.error_ty();
        };
        if !self.is_error(ret_ty) {
            self.results.record_method(
                expr,
                ResolvedMethod {
                    kind: MethodKind::TraitBound {
                        trait_def: trait_ref.trait_def,
                        method: method.def,
                        receiver: receiver_kind,
                    },
                    receiver_adjust: adjust,
                    receiver_ty: self_ty,
                    receiver_form: ReceiverForm::Place,
                    receiver_derefs: 0,
                    array_to_slice: false,
                    arg_tys: params,
                    ret_ty,
                },
            );
        }
        ret_ty
    }

    pub(super) fn method_type_arguments(
        &mut self,
        generic_args: Option<seal_ty::SubstsId>,
        expected: usize,
        span: Span,
    ) -> Option<Vec<TyId>> {
        let generic_args = generic_args?;
        let arguments = self.hir.ty_ctxt().substs_args(generic_args);
        if arguments.len() == expected {
            Some(arguments)
        } else {
            self.error(
                codes::E3044,
                span,
                format!(
                    "this method takes {expected} type argument(s), but {} were supplied",
                    arguments.len()
                ),
            )
            .help("supply exactly one type argument per method parameter, or omit the list")
            .emit();
            None
        }
    }

    fn receiver_adjustment(
        &mut self,
        receiver: HirExprId,
        receiver_ty: TyId,
        base_ty: TyId,
        expected: TyId,
        receiver_kind: ReceiverKind,
        name: Symbol,
    ) -> ReceiverAdjust {
        if receiver_ty == expected {
            return ReceiverAdjust::None;
        }
        if receiver_ty == base_ty {
            return match receiver_kind {
                ReceiverKind::Value => ReceiverAdjust::None,
                ReceiverKind::Shared => ReceiverAdjust::SharedBorrow,
                ReceiverKind::Exclusive => ReceiverAdjust::ExclusiveBorrow,
            };
        }
        if receiver_kind == ReceiverKind::Shared
            && matches!(
                self.hir.ty_ctxt().kind(receiver_ty),
                TyKind::Ref {
                    kind: RefKind::Exclusive,
                    pointee,
                } if pointee == base_ty
            )
            && expected == self.ref_ty(RefKind::Shared, base_ty)
        {
            return ReceiverAdjust::SharedReborrow;
        }
        self.error(
            codes::E3031,
            self.hir.expr(receiver).span,
            format!(
                "method `{}` expects `{}`, but the receiver has type `{}`",
                self.interner.resolve(name),
                self.ty_text(expected),
                self.ty_text(receiver_ty)
            ),
        )
        .help(
            "adjust the receiver to match the method signature; a `*mut T` receiver can be reborrowed as `*T` for a read-only method",
        )
        .emit();
        ReceiverAdjust::None
    }

    fn check_receiver(
        &mut self,
        receiver: HirExprId,
        base_ty: TyId,
        receiver_place: Option<&ResolvedPlace>,
        adjust: ReceiverAdjust,
        receiver_kind: ReceiverKind,
        name: Symbol,
    ) {
        if receiver_kind == ReceiverKind::Value
            && adjust == ReceiverAdjust::None
            && receiver_place.is_some_and(|place| place.contains_deref())
            && !self.is_copy(base_ty)
        {
            self.report_move_out_of_reference(base_ty, self.hir.expr(receiver).span);
        }
        if adjust == ReceiverAdjust::ExclusiveBorrow
            && !self.receiver_place_is_writable(receiver_place)
        {
            self.error(
                codes::E3029,
                self.hir.expr(receiver).span,
                format!(
                    "method `{}` needs to modify its receiver",
                    self.interner.resolve(name)
                ),
            )
            .help("declare the binding with `let mut`, or pass an exclusive reference")
            .emit();
        }
    }

    fn receiver_place_is_writable(&self, place: Option<&ResolvedPlace>) -> bool {
        match place {
            Some(place) => {
                self.root_is_writable(place.root) || place.deref == Some(RefKind::Exclusive)
            }
            None => true,
        }
    }

    fn resolve_method(
        &self,
        receiver: HirExprId,
        receiver_ty: TyId,
        receiver_place: Option<&crate::results::ResolvedPlace>,
        name: Symbol,
        name_span: Span,
        original_ty: TyId,
    ) -> Option<Resolution> {
        let mut ty = receiver_ty;
        let mut derefs = 0;
        let mut refs = Vec::new();
        let mut slice_ref_kind = None;
        let (owner, base_ty, form) = loop {
            match self.hir.ty_ctxt().kind(ty) {
                TyKind::Ref { kind, pointee } => {
                    if let TyKind::Slice { element: _ } = self.hir.ty_ctxt().kind(pointee) {
                        slice_ref_kind = Some(kind);
                        break (MethodOwner::Slice, pointee, ReceiverForm::Value);
                    }
                    refs.push(kind);
                    derefs += 1;
                    if derefs > MAX_RECEIVER_DEREFS {
                        self.error(
                            codes::E3027,
                            name_span,
                            "method receiver has too many reference layers",
                        )
                        .help("dereference the receiver explicitly before calling the method")
                        .emit();
                        return None;
                    }
                    ty = pointee;
                }
                TyKind::Int(_) => break (MethodOwner::Int, ty, ReceiverForm::Place),
                TyKind::Uint(_) => break (MethodOwner::Uint, ty, ReceiverForm::Place),
                TyKind::Float(_) => break (MethodOwner::Float, ty, ReceiverForm::Place),
                TyKind::Bool => break (MethodOwner::Bool, ty, ReceiverForm::Place),
                TyKind::Char => break (MethodOwner::Char, ty, ReceiverForm::Place),
                TyKind::Array { .. } => break (MethodOwner::Array, ty, ReceiverForm::Place),
                TyKind::Str => break (MethodOwner::Str, ty, ReceiverForm::Value),
                TyKind::RawPtr { .. } => break (MethodOwner::RawPtr, ty, ReceiverForm::Value),
                TyKind::Error => return None,
                _ => {
                    self.report_missing_method(original_ty, name, name_span);
                    return None;
                }
            }
        };
        let _ = receiver;

        let method_name = self.interner.resolve(name);
        let mut array_to_slice = false;
        let id = match seal_methods::find(owner, method_name) {
            Some(id) => id,
            None if owner == MethodOwner::Array => {
                match seal_methods::find(MethodOwner::Slice, method_name) {
                    Some(id) => {
                        array_to_slice = true;
                        id
                    }
                    None => {
                        self.report_missing_method(original_ty, name, name_span);
                        return None;
                    }
                }
            }
            None => {
                self.report_missing_method(original_ty, name, name_span);
                return None;
            }
        };
        let def = id.def();
        if def.unsafe_op && self.unsafe_depth == 0 {
            self.error(
                codes::E3070,
                span_for_method(self.hir, receiver),
                format!("calling `{}` requires an `unsafe` block", def.name),
            )
            .help("wrap the call in `unsafe { ... }`")
            .emit();
        }
        if def.intrinsic == IntrinsicOp::SliceAsStrUnchecked
            && !matches!(
                self.hir.ty_ctxt().kind(base_ty),
                TyKind::Slice { element } if element == self.u8_ty()
            )
        {
            self.error(
                codes::E3075,
                span_for_method(self.hir, receiver),
                "`as_str_unchecked` requires a byte slice",
            )
            .help("only `*[u8]` can become a `*str`")
            .emit();
        }
        let element_ty = match self.hir.ty_ctxt().kind(base_ty) {
            TyKind::Array { element, .. } => element,
            TyKind::Slice { element } => element,
            TyKind::RawPtr { pointee, .. } => pointee,
            _ => base_ty,
        };
        if def.owner == MethodOwner::RawPtr {
            if raw_pointer_requires_mut(def.intrinsic)
                && matches!(
                    self.hir.ty_ctxt().kind(base_ty),
                    TyKind::RawPtr {
                        kind: seal_ty::RawPtrKind::Const,
                        ..
                    }
                )
            {
                self.error(
                    codes::E3075,
                    span_for_method(self.hir, receiver),
                    format!("`{}` writes through a `*raw T` pointer", def.name),
                )
                .help("declare the pointer as `*raw mut T` to allow writes")
                .emit();
            }
            if raw_pointer_requires_sized(def.intrinsic) && !self.is_sized(element_ty) {
                self.error(
                    codes::E3075,
                    span_for_method(self.hir, receiver),
                    format!(
                        "`{}` requires a sized pointee, but `{}` is unsized",
                        def.name,
                        self.ty_text(element_ty)
                    ),
                )
                .help("call `data()` first to discard the metadata")
                .emit();
            }
            if def.intrinsic == IntrinsicOp::PtrLen
                && !matches!(
                    self.hir.ty_ctxt().kind(element_ty),
                    TyKind::Slice { .. } | TyKind::Str
                )
            {
                self.error(
                    codes::E3075,
                    span_for_method(self.hir, receiver),
                    "`len` requires a raw pointer to a slice or `str`",
                )
                .help("a thin raw pointer has no length metadata")
                .emit();
            }
            if def.intrinsic == IntrinsicOp::PtrWithLen && !self.is_sized(element_ty) {
                self.error(
                    codes::E3075,
                    span_for_method(self.hir, receiver),
                    format!(
                        "`with_len` requires a thin pointer to a sized pointee, but `{}` is unsized",
                        self.ty_text(element_ty)
                    ),
                )
                .help("call `data()` first to discard the existing metadata")
                .emit();
            }
            if def.intrinsic == IntrinsicOp::PtrAsStr
                && !matches!(
                    self.hir.ty_ctxt().kind(element_ty),
                    TyKind::Slice { element } if element == self.u8_ty()
                )
            {
                self.error(
                    codes::E3075,
                    span_for_method(self.hir, receiver),
                    "`as_str` requires a raw pointer to `[u8]`",
                )
                .help("only a byte slice pointer can become a raw string pointer")
                .emit();
            }
        }
        if def.receiver == ReceiverMode::Exclusive && slice_ref_kind == Some(RefKind::Shared) {
            self.error(
                codes::E3029,
                self.hir.expr(receiver).span,
                format!("method `{}` cannot write through a shared slice", def.name),
            )
            .help("use a `*mut [T]` receiver, or declare the slice binding as mutable")
            .emit();
        }
        if def.receiver == ReceiverMode::Exclusive && derefs > 0 && refs.contains(&RefKind::Shared)
        {
            self.error(
                codes::E3029,
                self.hir.expr(receiver).span,
                format!(
                    "method `{}` cannot write through a shared reference",
                    def.name
                ),
            )
            .help("dereference a `*mut T` receiver, or declare the binding with `let mut`")
            .emit();
        }
        if def.receiver == ReceiverMode::Exclusive && derefs == 0 && form == ReceiverForm::Place {
            if let Some(place) = receiver_place {
                let writable =
                    self.root_is_writable(place.root) || place.deref == Some(RefKind::Exclusive);
                if !writable {
                    self.error(
                        codes::E3029,
                        self.hir.expr(receiver).span,
                        format!(
                            "method `{}` modifies `{}`, which is not a mutable place",
                            def.name,
                            self.ty_text(base_ty)
                        ),
                    )
                    .help("declare the binding with `let mut`, or use a `*mut T` receiver")
                    .emit();
                }
            }
        }

        Some(Resolution {
            id,
            def,
            receiver_ty: base_ty,
            form,
            derefs,
            array_to_slice,
            element_ty,
        })
    }

    fn receiver_is_writable(
        &self,
        receiver: HirExprId,
        place: Option<&crate::results::ResolvedPlace>,
        resolution: &Resolution,
    ) -> bool {
        if resolution.form == ReceiverForm::Value {
            return true;
        }
        if resolution.derefs > 0 {
            return true;
        }
        match place {
            Some(place) => {
                self.root_is_writable(place.root) || place.deref == Some(RefKind::Exclusive)
            }
            None => {
                // A temporary has its own storage, which exists only for this
                // call. It can be modified even though no effect is observable.
                let _ = receiver;
                true
            }
        }
    }

    /// Reports a direct call to `Drop::drop`.
    ///
    /// The compiler invokes the destructor when the value's scope ends. Naming
    /// it would run it while the value is still live, releasing the same
    /// resource twice, so the call is rejected instead.
    pub(super) fn report_drop_call(&self, span: Span) {
        self.error(
            codes::E3065,
            span,
            "the destructor cannot be called directly",
        )
        .help("the compiler runs `drop` when the value goes out of scope; remove this call")
        .emit();
    }

    pub(super) fn report_missing_method(&self, receiver_ty: TyId, name: Symbol, span: Span) {
        let name = self.interner.resolve(name);
        self.error(
            codes::E3027,
            span,
            format!(
                "no method named `{name}` is available for `{}`",
                self.ty_text(receiver_ty)
            ),
        )
        .help("check the method name, or dereference the receiver to the type that defines it")
        .emit();
    }

    fn param_ty(&self, kind: ParamKind, resolution: &Resolution) -> TyId {
        match kind {
            ParamKind::SelfValue => resolution.element_ty,
            ParamKind::Element => resolution.element_ty,
            ParamKind::Usize => self.usize_ty(),
            ParamKind::I32 => self.i32_ty(),
            ParamKind::U32 => self.u32_ty(),
            ParamKind::Isize => self.isize_ty(),
            ParamKind::Pointer => {
                let tcx = self.hir.ty_ctxt();
                match tcx.kind(resolution.receiver_ty) {
                    TyKind::RawPtr { kind, pointee } => {
                        // `copy` reads its source, so it accepts a read-only
                        // pointer to the same pointee. `offset_from` compares
                        // pointers of one type and keeps the receiver's kind.
                        let kind = if matches!(
                            resolution.def.intrinsic,
                            IntrinsicOp::PtrCopy | IntrinsicOp::PtrCopyNonOverlapping
                        ) {
                            seal_ty::RawPtrKind::Const
                        } else {
                            kind
                        };
                        tcx.raw_ptr_ty_with(kind, pointee)
                    }
                    _ => resolution.receiver_ty,
                }
            }
            ParamKind::Bool => self.bool_ty(),
        }
    }

    fn return_ty(&self, resolution: &Resolution) -> TyId {
        match resolution.def.ret {
            ReturnKind::Same => resolution.element_ty,
            ReturnKind::Bool => self.bool_ty(),
            ReturnKind::Usize => self.usize_ty(),
            ReturnKind::I32 => self.i32_ty(),
            ReturnKind::U32 => self.u32_ty(),
            ReturnKind::Isize => self.isize_ty(),
            ReturnKind::Unit => self.unit(),
            ReturnKind::Pointer if resolution.def.intrinsic == IntrinsicOp::PtrData => {
                self.data_pointer_ty(resolution.receiver_ty)
            }
            ReturnKind::Pointer => resolution.receiver_ty,
            ReturnKind::SlicePointer => self.slice_pointer_ty(resolution.receiver_ty),
            ReturnKind::StrPointer => self.str_pointer_ty(resolution.receiver_ty),
            ReturnKind::StrReference => self.str_ref_ty(),
            ReturnKind::SliceReference => {
                let slice = self.slice_ty(self.u8_ty());
                self.ref_ty(RefKind::Shared, slice)
            }
            ReturnKind::Char => self.char_ty(),
        }
    }

    /// Returns `*raw [T]` with the receiver's pointer kind.
    fn slice_pointer_ty(&self, receiver_ty: TyId) -> TyId {
        let tcx = self.hir.ty_ctxt();
        let TyKind::RawPtr { kind, pointee } = tcx.kind(receiver_ty) else {
            return self.error_ty();
        };
        if !self.is_sized(pointee) {
            return self.error_ty();
        }
        let slice = tcx.slice_ty(pointee);
        tcx.raw_ptr_ty_with(kind, slice)
    }

    /// Returns `*raw str` with the receiver's pointer kind.
    fn str_pointer_ty(&self, receiver_ty: TyId) -> TyId {
        let tcx = self.hir.ty_ctxt();
        let TyKind::RawPtr { kind, pointee } = tcx.kind(receiver_ty) else {
            return self.error_ty();
        };
        if !matches!(
            tcx.kind(pointee),
            TyKind::Slice { element } if element == self.u8_ty()
        ) {
            return self.error_ty();
        }
        tcx.raw_ptr_ty_with(kind, tcx.str_ty())
    }

    /// Returns the thin pointer type produced by `data()`.
    ///
    /// A slice keeps its element type, a string yields `u8`, and a trait object
    /// yields `()`: the erased representation has no useful pointee type, and
    /// the address is only an opaque handle for byte-level work.
    fn data_pointer_ty(&self, receiver_ty: TyId) -> TyId {
        let tcx = self.hir.ty_ctxt();
        let TyKind::RawPtr { kind, pointee } = tcx.kind(receiver_ty) else {
            return receiver_ty;
        };
        let thin = match tcx.kind(pointee) {
            TyKind::Slice { element } => element,
            TyKind::Str => self.u8_ty(),
            TyKind::Dyn { .. } => self.unit(),
            _ => pointee,
        };
        tcx.raw_ptr_ty_with(kind, thin)
    }
}

/// Returns whether an intrinsic reads or writes whole `T` values and therefore
/// requires a sized pointee.
fn raw_pointer_requires_sized(intrinsic: IntrinsicOp) -> bool {
    matches!(
        intrinsic,
        IntrinsicOp::PtrAdd
            | IntrinsicOp::PtrSub
            | IntrinsicOp::PtrOffset
            | IntrinsicOp::PtrByteAdd
            | IntrinsicOp::PtrByteOffset
            | IntrinsicOp::PtrOffsetFrom
            | IntrinsicOp::PtrWrappingAdd
            | IntrinsicOp::PtrWrappingSub
            | IntrinsicOp::PtrWrappingOffset
            | IntrinsicOp::PtrRead
            | IntrinsicOp::PtrWrite
            | IntrinsicOp::PtrReadUnaligned
            | IntrinsicOp::PtrWriteUnaligned
            | IntrinsicOp::PtrCopy
            | IntrinsicOp::PtrCopyNonOverlapping
    )
}

/// Returns whether an intrinsic writes through its receiver and therefore
/// requires a `*raw mut T`.
fn raw_pointer_requires_mut(intrinsic: IntrinsicOp) -> bool {
    matches!(
        intrinsic,
        IntrinsicOp::PtrWrite
            | IntrinsicOp::PtrWriteUnaligned
            | IntrinsicOp::PtrCopy
            | IntrinsicOp::PtrCopyNonOverlapping
    )
}

/// Returns the receiver's source span.
fn span_for_method(hir: &seal_hir::Hir, receiver: HirExprId) -> Span {
    hir.expr(receiver).span
}

/// One trait impl that could provide the called method.
struct TraitCandidate {
    trait_def: DefId,
    kind: TraitCandidateKind,
    seed: FxHashMap<seal_ty::ParamTyId, TyId>,
    /// The self type matched against the receiver, used for receiver checks.
    self_ty: TyId,
}

impl TraitCandidate {
    /// Turns the selected candidate into the concrete or symbolic call shape.
    fn into_user_method(self) -> UserMethod {
        match self.kind {
            TraitCandidateKind::Provided { function, receiver } => UserMethod::Concrete {
                function,
                receiver,
                source: MethodSource::Trait(self.trait_def),
                self_ty: self.self_ty,
                seed: self.seed,
            },
            TraitCandidateKind::Default { trait_ref, method } => UserMethod::Bound {
                trait_ref,
                method,
                self_ty: self.self_ty,
            },
        }
    }
}

/// How one matching trait impl provides the called method.
enum TraitCandidateKind {
    /// The impl declares a concrete function.
    Provided {
        function: DefId,
        receiver: ReceiverKind,
    },
    /// The impl omits the method and the trait supplies a default body.
    Default {
        trait_ref: seal_ty::TraitRef,
        method: seal_ty::TraitMethodDef,
    },
}

/// The user-defined resolution chosen for a method call.
enum UserMethod {
    /// A method reached through an impl block.
    Concrete {
        function: DefId,
        receiver: ReceiverKind,
        source: MethodSource,
        self_ty: TyId,
        seed: FxHashMap<seal_ty::ParamTyId, TyId>,
    },
    /// A method reached through a bound on a type parameter.
    Bound {
        trait_ref: seal_ty::TraitRef,
        method: seal_ty::TraitMethodDef,
        self_ty: TyId,
    },
}

/// Where a user method was declared.
#[derive(Clone, Copy)]
enum MethodSource {
    /// An inherent `impl Type` block.
    Inherent,
    /// An `impl Trait for Type` block, with the trait definition.
    Trait(DefId),
}

/// The result of looking for a user method before the intrinsic catalog.
enum UserLookup {
    /// The receiver is not a struct, so intrinsic lookup applies.
    NotAStruct,
    /// The call resolved to a user method.
    Method(UserMethod),
    /// The receiver is a struct and the error was already reported.
    Rejected,
}

/// The checked pieces of a user method call.
struct UserCall<'a> {
    expr: HirExprId,
    receiver: HirExprId,
    receiver_ty: TyId,
    receiver_place: Option<ResolvedPlace>,
    target: UserMethod,
    name: Symbol,
    name_span: Span,
    generic_args: Option<seal_ty::SubstsId>,
    args: &'a [HirExprId],
    span: Span,
    expected: Option<TyId>,
}

/// One concrete impl method call.
struct ConcreteCall<'a> {
    expr: HirExprId,
    receiver: HirExprId,
    receiver_ty: TyId,
    receiver_place: Option<ResolvedPlace>,
    function: DefId,
    receiver_kind: ReceiverKind,
    source: MethodSource,
    self_ty: TyId,
    seed: FxHashMap<seal_ty::ParamTyId, TyId>,
    name: Symbol,
    generic_args: Option<seal_ty::SubstsId>,
    args: &'a [HirExprId],
    span: Span,
    expected: Option<TyId>,
}

/// One trait-bound method call.
struct BoundCall<'a> {
    expr: HirExprId,
    receiver: HirExprId,
    receiver_ty: TyId,
    receiver_place: Option<ResolvedPlace>,
    trait_ref: seal_ty::TraitRef,
    method: seal_ty::TraitMethodDef,
    self_ty: TyId,
    name: Symbol,
    generic_args: Option<seal_ty::SubstsId>,
    args: &'a [HirExprId],
    span: Span,
    expected: Option<TyId>,
}

/// Returns `true` when the receiver read through `Index` must be resolved
/// again through `IndexMut` because the selected method needs an exclusive
/// receiver.
fn needs_exclusive_reindex(place: &ResolvedPlace) -> bool {
    place.projections.iter().any(|projection| {
        matches!(
            projection,
            crate::results::PlaceProjection::CustomIndex { mutable: false, .. }
        )
    })
}

fn user_method_receiver(method: &UserMethod) -> Option<ReceiverKind> {
    match method {
        UserMethod::Concrete { receiver, .. } => Some(*receiver),
        UserMethod::Bound { method, .. } => method.receiver.map(trait_receiver_kind),
    }
}

pub(super) fn trait_receiver_kind(receiver: seal_ty::TraitMethodReceiver) -> ReceiverKind {
    match receiver.reference {
        None => ReceiverKind::Value,
        Some(RefKind::Shared) => ReceiverKind::Shared,
        Some(RefKind::Exclusive) => ReceiverKind::Exclusive,
    }
}

struct Resolution {
    id: MethodId,
    def: &'static MethodDef,
    receiver_ty: TyId,
    form: ReceiverForm,
    derefs: usize,
    array_to_slice: bool,
    /// The scalar type for scalar owners, or the element type for sequences.
    element_ty: TyId,
}
