//! Protocol-based indexing.
//!
//! Arrays and slices keep their compiler-owned indexing path. Every other base
//! resolves through the installed `Index` protocol for reads and shared
//! borrows. Candidate search follows reference layers from the outside in, so
//! an implementation on `*Grid` wins over one on `Grid`, matching ordinary
//! method lookup. The resolved call is recorded once for later MIR lowering.

use seal_def::DefId;
use seal_error_codes as codes;
use seal_hash::FxHashMap;
use seal_hir::{HirExprId, Protocol, ProtocolMethod, ReceiverKind};
use seal_layout::{VtableLayout, VtableSlot};
use seal_span::Span;
use seal_symbol::Symbol;
use seal_ty::{ParamTyId, RefKind, TraitImplDef, TraitMethodDef, TraitRef, TyId, TyKind};

use super::Checker;
use super::PlaceMode;
use super::flow::Flow;
use super::method::trait_receiver_kind;
use super::trait_solver::match_type;
use crate::results::{
    IndexProtocol, MethodKind, PlaceProjection, ProtocolCall, ReceiverAdjust, ReceiverForm,
    ResolvedMethod,
};

/// The checked pieces of one index expression.
#[derive(Clone, Copy)]
pub(super) struct IndexAccess {
    /// The index expression itself.
    pub(super) expr: HirExprId,
    /// The indexed place.
    pub(super) base: HirExprId,
    /// The type of the indexed place before protocol lookup.
    pub(super) base_ty: TyId,
    /// The expression inside the brackets.
    pub(super) index: HirExprId,
    /// How the selected place is used.
    pub(super) mode: PlaceMode,
    /// The complete index expression's span.
    pub(super) span: Span,
    /// The type expected by the surrounding context, when any.
    pub(super) expected: Option<TyId>,
}

/// How a selected protocol candidate dispatches its method.
#[derive(Clone, Debug)]
pub(super) enum CandidateTarget {
    /// A concrete impl function.
    Impl {
        function: DefId,
        receiver: ReceiverKind,
        seed: FxHashMap<ParamTyId, TyId>,
    },
    /// A trait method reached through a bound on a type parameter.
    Bound {
        method: TraitMethodDef,
        receiver: ReceiverKind,
    },
    /// A method dispatched through a trait-object vtable.
    Dynamic {
        method: TraitMethodDef,
        receiver: ReceiverKind,
        slot: VtableSlot,
    },
}

/// One implementation or bound that can satisfy a built-in protocol.
#[derive(Clone, Debug)]
pub(super) struct ProtocolCandidate {
    pub(super) trait_ref: TraitRef,
    pub(super) self_ty: TyId,
    pub(super) target: CandidateTarget,
}

impl Checker<'_> {
    /// Resolves a custom index expression for a read, shared borrow, write, or
    /// exclusive borrow.
    ///
    /// Returns `Some` when protocol resolution handled the expression, even
    /// when it reported an error. `None` is reserved for the built-in path;
    /// `check_index` only calls this after it has ruled the built-in path out.
    pub(super) fn resolve_index(&mut self, access: IndexAccess, flow: &mut Flow) -> Option<TyId> {
        let IndexAccess {
            expr,
            base,
            base_ty,
            index,
            mode,
            span,
            expected,
        } = access;
        let protocol = match mode {
            PlaceMode::Read | PlaceMode::Borrow => Protocol::Index,
            PlaceMode::Write | PlaceMode::BorrowExclusive => Protocol::IndexMut,
        };

        // HIR already reported `E2013` for a non-place base. Type the index
        // expression for the fixture invariant, but do not pile a second
        // diagnostic on top of the structural error.
        if self.results.place(base).is_none() {
            self.infer_expr(index, None, flow);
            return Some(self.error_ty());
        }
        if self.is_error(base_ty) {
            self.infer_expr(index, None, flow);
            return Some(self.error_ty());
        }

        let method = match protocol {
            Protocol::Index => ProtocolMethod::Index,
            Protocol::IndexMut => ProtocolMethod::IndexMut,
            Protocol::Iterator | Protocol::IntoIterator => {
                unreachable!("index resolution only uses Index and IndexMut")
            }
        };
        let trait_def = self.hir.protocol_trait(protocol);
        let method_def = self.hir.protocol_defs().method(protocol, method);
        let method_name = self.hir.definitions().name(method_def);

        for layer_ty in self.reference_layers(base_ty) {
            let candidates = self.protocol_candidates(layer_ty, trait_def, method_name, true);
            if candidates.is_empty() {
                continue;
            }
            let Some((candidate, index_checked)) =
                self.select_index_candidate(candidates, protocol, index, span, flow)
            else {
                return Some(self.error_ty());
            };
            let access = IndexAccess {
                expr,
                base,
                base_ty,
                index,
                mode,
                span,
                expected,
            };
            return Some(self.finish_index(access, candidate, protocol, index_checked, flow));
        }

        self.infer_expr(index, None, flow);
        if protocol == Protocol::IndexMut {
            self.report_index_not_mutable(base_ty, span);
        } else {
            self.report_cannot_index(base_ty, span);
        }
        Some(self.error_ty())
    }

    /// Returns the base type followed by each pointee reached by one more
    /// reference layer.
    pub(super) fn reference_layers(&self, ty: TyId) -> Vec<TyId> {
        let mut layers = vec![ty];
        let mut current = ty;
        while let TyKind::Ref { pointee, .. } = self.hir.ty_ctxt().kind(current) {
            layers.push(pointee);
            current = pointee;
        }
        layers
    }

    /// Collects every protocol candidate whose self type is `layer_ty`.
    ///
    /// `allow_dynamic` includes a trait-object receiver when the layer is a
    /// reference to the protocol's principal trait object. Iteration rejects
    /// dynamic sources by object safety, so it passes `false`.
    pub(super) fn protocol_candidates(
        &mut self,
        layer_ty: TyId,
        trait_def: DefId,
        method_name: Symbol,
        allow_dynamic: bool,
    ) -> Vec<ProtocolCandidate> {
        let mut candidates = Vec::new();
        for implementation in self.hir.ty_ctxt().trait_impls_matching(layer_ty) {
            if let Some(candidate) =
                self.impl_candidate(&implementation, layer_ty, trait_def, method_name)
            {
                candidates.push(candidate);
            }
        }

        if let TyKind::Param(param) = self.hir.ty_ctxt().kind(layer_ty) {
            let visible = self.visible_traits(param);
            for trait_ref in visible {
                if trait_ref.trait_def != trait_def {
                    continue;
                }
                let Some(definition) = self.hir.trait_definition(trait_def) else {
                    continue;
                };
                let Some(method) = definition.method(method_name).cloned() else {
                    continue;
                };
                let Some(declared_receiver) = method.receiver else {
                    continue;
                };
                candidates.push(ProtocolCandidate {
                    trait_ref,
                    self_ty: layer_ty,
                    target: CandidateTarget::Bound {
                        method,
                        receiver: trait_receiver_kind(declared_receiver),
                    },
                });
            }
        }

        if allow_dynamic
            && let Some(candidate) = self.dynamic_candidate(layer_ty, trait_def, method_name)
        {
            candidates.push(candidate);
        }

        candidates
    }

    /// Instantiates one concrete impl header against `layer_ty`.
    fn impl_candidate(
        &mut self,
        implementation: &TraitImplDef,
        layer_ty: TyId,
        trait_def: DefId,
        method_name: Symbol,
    ) -> Option<ProtocolCandidate> {
        if implementation.trait_ref.trait_def != trait_def {
            return None;
        }
        let mut seed = FxHashMap::default();
        if !match_type(
            self.hir,
            implementation.trait_ref.self_ty,
            layer_ty,
            &mut seed,
        ) {
            return None;
        }

        let mut arguments = Vec::with_capacity(implementation.generics.params.len());
        for parameter in &implementation.generics.params {
            let argument = seed
                .get(&parameter.param)
                .copied()
                .unwrap_or_else(|| self.infer.fresh_ty_var(self.hir.ty_ctxt()));
            seed.insert(parameter.param, argument);
            arguments.push(argument);
        }
        let substs = self.hir.ty_ctxt().substs(&arguments);
        let trait_arguments: Vec<TyId> = self
            .hir
            .ty_ctxt()
            .substs_args(implementation.trait_ref.args)
            .into_iter()
            .map(|argument| self.hir.ty_ctxt().substitute(argument, substs))
            .collect();
        let function = implementation.method(method_name)?;
        let receiver = self.hir.receiver(function)?;

        Some(ProtocolCandidate {
            trait_ref: TraitRef::new(
                trait_def,
                layer_ty,
                self.hir.ty_ctxt().substs(&trait_arguments),
            ),
            self_ty: layer_ty,
            target: CandidateTarget::Impl {
                function,
                receiver,
                seed,
            },
        })
    }

    /// Recognizes a protocol trait object reached through a reference.
    fn dynamic_candidate(
        &self,
        layer_ty: TyId,
        trait_def: DefId,
        method_name: Symbol,
    ) -> Option<ProtocolCandidate> {
        let TyKind::Ref { pointee, .. } = self.hir.ty_ctxt().kind(layer_ty) else {
            return None;
        };
        let TyKind::Dyn {
            trait_def: object_trait,
            args,
        } = self.hir.ty_ctxt().kind(pointee)
        else {
            return None;
        };
        if object_trait != trait_def {
            return None;
        }
        let definition = self.hir.trait_definition(trait_def)?;
        let method = definition.method(method_name)?.clone();
        let receiver = trait_receiver_kind(method.receiver?);
        let trait_ref = TraitRef::new(trait_def, pointee, args);
        let slot = VtableLayout::new(self.hir.ty_ctxt(), trait_ref)
            .ok()?
            .slot_of(method.def)?;
        Some(ProtocolCandidate {
            trait_ref,
            self_ty: pointee,
            target: CandidateTarget::Dynamic {
                method,
                receiver,
                slot,
            },
        })
    }

    /// Chooses one candidate, using the index expression when the type has
    /// several implementations with different index types.
    ///
    /// The returned flag says whether the index expression was already checked.
    fn select_index_candidate(
        &mut self,
        mut candidates: Vec<ProtocolCandidate>,
        protocol: Protocol,
        index: HirExprId,
        span: Span,
        flow: &mut Flow,
    ) -> Option<(ProtocolCandidate, bool)> {
        if candidates.len() == 1 {
            return candidates.pop().map(|candidate| (candidate, false));
        }

        let index_types: Vec<TyId> = candidates
            .iter()
            .filter_map(|candidate| self.candidate_trait_args(candidate).map(|args| args[0]))
            .collect();
        let resolved: Vec<TyId> = index_types
            .iter()
            .map(|index_ty| self.resolve_deep(*index_ty))
            .collect();
        let all_same = !resolved.is_empty()
            && resolved.windows(2).all(|pair| pair[0] == pair[1])
            && !resolved
                .iter()
                .any(|index_ty| self.contains_infer(*index_ty));
        if all_same {
            self.infer_expr(index, None, flow);
            self.report_ambiguous_index(
                protocol,
                self.candidate_base_ty(&candidates[0]),
                span,
                &candidates,
            );
            return None;
        }

        let actual = self.infer_expr(index, None, flow);
        let mut matching = Vec::new();
        for (position, candidate) in candidates.iter().enumerate() {
            let Some(args) = self.candidate_trait_args(candidate) else {
                continue;
            };
            let mut trial = self.infer.clone();
            if trial.unify(self.hir.ty_ctxt(), args[0], actual).is_ok() {
                matching.push(position);
            }
        }

        match matching.as_slice() {
            [position] => {
                let candidate = candidates.swap_remove(*position);
                let args = self.candidate_trait_args(&candidate)?;
                let _ = self.infer.unify(self.hir.ty_ctxt(), args[0], actual);
                Some((candidate, true))
            }
            [] => {
                let mut unique: Vec<TyId> = Vec::new();
                for index_ty in resolved {
                    if !self.is_error(index_ty) && !unique.contains(&index_ty) {
                        unique.push(index_ty);
                    }
                }
                if unique.len() == 1 {
                    self.mismatch(self.hir.expr(index).span, unique[0], actual);
                } else {
                    self.report_ambiguous_index(
                        protocol,
                        self.candidate_base_ty(&candidates[0]),
                        span,
                        &candidates,
                    );
                }
                None
            }
            _ => {
                self.report_ambiguous_index(
                    protocol,
                    self.candidate_base_ty(&candidates[0]),
                    span,
                    &candidates,
                );
                None
            }
        }
    }

    pub(super) fn candidate_trait_args(&self, candidate: &ProtocolCandidate) -> Option<Vec<TyId>> {
        let args = self.hir.ty_ctxt().substs_args(candidate.trait_ref.args);
        (!args.is_empty()).then_some(args)
    }

    /// Rebuilds a selected candidate for another method of the same protocol.
    ///
    /// The candidate's instantiation is preserved, so inference variables that
    /// tie a protocol type argument to the impl stay shared across calls.
    pub(super) fn with_protocol_method(
        &self,
        candidate: &ProtocolCandidate,
        method_name: Symbol,
    ) -> Option<ProtocolCandidate> {
        let target = match &candidate.target {
            CandidateTarget::Impl { function, seed, .. } => {
                let implementations = self.hir.trait_impls();
                let implementation = implementations.iter().find(|implementation| {
                    implementation
                        .methods
                        .iter()
                        .any(|(_, candidate_function)| candidate_function == function)
                })?;
                let function = implementation.method(method_name)?;
                let receiver = self.hir.receiver(function)?;
                CandidateTarget::Impl {
                    function,
                    receiver,
                    seed: seed.clone(),
                }
            }
            CandidateTarget::Bound { .. } => {
                let declaration = self.hir.trait_definition(candidate.trait_ref.trait_def)?;
                let method = declaration.method(method_name)?.clone();
                let receiver = trait_receiver_kind(method.receiver?);
                CandidateTarget::Bound { method, receiver }
            }
            CandidateTarget::Dynamic { .. } => {
                let declaration = self.hir.trait_definition(candidate.trait_ref.trait_def)?;
                let method = declaration.method(method_name)?.clone();
                let receiver = trait_receiver_kind(method.receiver?);
                let slot = VtableLayout::new(self.hir.ty_ctxt(), candidate.trait_ref)
                    .ok()?
                    .slot_of(method.def)?;
                CandidateTarget::Dynamic {
                    method,
                    receiver,
                    slot,
                }
            }
        };
        Some(ProtocolCandidate {
            trait_ref: candidate.trait_ref,
            self_ty: candidate.self_ty,
            target,
        })
    }

    pub(super) fn candidate_base_ty(&self, candidate: &ProtocolCandidate) -> TyId {
        candidate.trait_ref.self_ty
    }

    /// Checks the index argument, materializes the protocol call, and records
    /// the resulting custom place.
    fn finish_index(
        &mut self,
        access: IndexAccess,
        candidate: ProtocolCandidate,
        protocol: Protocol,
        index_checked: bool,
        flow: &mut Flow,
    ) -> TyId {
        let IndexAccess {
            expr,
            base,
            base_ty,
            index,
            mode,
            span,
            expected,
        } = access;
        let Some(args) = self.candidate_trait_args(&candidate) else {
            return self.error_ty();
        };
        let index_ty = args[0];
        let element_ty = args[1];
        if !index_checked {
            self.check_expr(index, index_ty, flow);
        }
        if let Some(expected) = expected {
            let _ = self.infer.unify(self.hir.ty_ctxt(), expected, element_ty);
        }

        let resolved_index = self.resolve_deep(index_ty);
        let resolved_element = self.resolve_deep(element_ty);
        if (!self.is_error(resolved_index) && self.contains_infer(resolved_index))
            || (!self.is_error(resolved_element) && self.contains_infer(resolved_element))
        {
            self.report_unresolved_index(protocol, base_ty, span, resolved_index, resolved_element);
            return self.error_ty();
        }
        if self.is_error(resolved_index) || self.is_error(resolved_element) {
            return self.error_ty();
        }

        let Some(call) = self.build_protocol_call(&candidate, base_ty, span) else {
            return self.error_ty();
        };

        self.require_initialized(base, flow, span);
        if protocol == Protocol::IndexMut
            && call.method.receiver_adjust == ReceiverAdjust::ExclusiveBorrow
            && !self.borrow_is_writable(base)
        {
            self.report_mutable_borrow(base);
            return self.error_ty();
        }

        if mode == PlaceMode::Read && !self.checking_place_base && !self.is_copy(resolved_element) {
            self.report_move_out_of_reference(resolved_element, span);
        }

        if let Some(mut place) = self.results.place(base).cloned() {
            place.projections.push(PlaceProjection::CustomIndex {
                expr,
                index,
                mutable: protocol == Protocol::IndexMut,
            });
            place.ty = resolved_element;
            place.deref = Some(if protocol == Protocol::IndexMut {
                RefKind::Exclusive
            } else {
                RefKind::Shared
            });
            self.results.record_place(expr, place);
        }

        self.results.record_index_protocol(
            expr,
            IndexProtocol::Custom {
                mutable: protocol == Protocol::IndexMut,
                element_ty: resolved_element,
                call,
            },
        );
        resolved_element
    }

    /// Builds the recorded method call for a selected candidate.
    pub(super) fn build_protocol_call(
        &mut self,
        candidate: &ProtocolCandidate,
        base_ty: TyId,
        span: Span,
    ) -> Option<ProtocolCall> {
        let (receiver_adjust, receiver_derefs) = self.protocol_receiver_adjustment(
            base_ty,
            candidate.self_ty,
            receiver_kind(candidate),
            span,
        )?;
        match &candidate.target {
            CandidateTarget::Impl {
                function,
                receiver,
                seed,
            } => {
                let instantiated = self.instantiate_function(*function, seed, None, None);
                let args =
                    self.finish_arguments(&instantiated.generics, &instantiated.args, span)?;
                let substs = self.hir.ty_ctxt().substs(&args);
                let signature = self.signature(*function)?.clone();
                let params: Vec<TyId> = signature
                    .params
                    .iter()
                    .map(|parameter| self.substitute(*parameter, substs))
                    .collect();
                let ret_ty = self.substitute(signature.ret, substs);
                let instance = seal_ty::Instance::new(*function, substs);
                Some(ProtocolCall {
                    method: ResolvedMethod {
                        kind: MethodKind::Trait {
                            function: *function,
                            receiver: *receiver,
                            trait_def: candidate.trait_ref.trait_def,
                        },
                        receiver_adjust,
                        receiver_ty: candidate.self_ty,
                        receiver_form: ReceiverForm::Place,
                        receiver_derefs,
                        array_to_slice: false,
                        arg_tys: params.into_iter().skip(1).collect(),
                        ret_ty,
                    },
                    instance: Some(instance),
                    lowering_instance: Some(instance),
                })
            }
            CandidateTarget::Bound { method, receiver } => {
                let instantiated =
                    self.instantiate_trait_method(candidate.trait_ref, method, None, None)?;
                let args =
                    self.finish_arguments(&instantiated.generics, &instantiated.args, span)?;
                let substs = self.hir.ty_ctxt().substs(&args);
                let arg_tys = method
                    .signature
                    .params
                    .iter()
                    .map(|parameter| self.substitute(*parameter, substs))
                    .collect();
                let ret_ty = self.substitute(method.signature.ret, substs);
                let lowering_instance = seal_ty::Instance::new(method.def, substs);
                Some(ProtocolCall {
                    method: ResolvedMethod {
                        kind: MethodKind::TraitBound {
                            trait_def: candidate.trait_ref.trait_def,
                            method: method.def,
                            receiver: *receiver,
                        },
                        receiver_adjust,
                        receiver_ty: candidate.self_ty,
                        receiver_form: ReceiverForm::Place,
                        receiver_derefs,
                        array_to_slice: false,
                        arg_tys,
                        ret_ty,
                    },
                    instance: None,
                    lowering_instance: Some(lowering_instance),
                })
            }
            CandidateTarget::Dynamic {
                method,
                receiver,
                slot,
            } => {
                let instantiated =
                    self.instantiate_trait_method(candidate.trait_ref, method, None, None)?;
                let args =
                    self.finish_arguments(&instantiated.generics, &instantiated.args, span)?;
                let substs = self.hir.ty_ctxt().substs(&args);
                let arg_tys = method
                    .signature
                    .params
                    .iter()
                    .map(|parameter| self.substitute(*parameter, substs))
                    .collect();
                let ret_ty = self.substitute(method.signature.ret, substs);
                Some(ProtocolCall {
                    method: ResolvedMethod {
                        kind: MethodKind::Dynamic {
                            object_trait_ref: candidate.trait_ref,
                            method: method.def,
                            slot: *slot,
                            receiver: *receiver,
                        },
                        receiver_adjust,
                        receiver_ty: candidate.self_ty,
                        receiver_form: ReceiverForm::Place,
                        receiver_derefs,
                        array_to_slice: false,
                        arg_tys,
                        ret_ty,
                    },
                    instance: None,
                    lowering_instance: None,
                })
            }
        }
    }

    /// Chooses the receiver adjustment needed to pass the base expression to
    /// the selected protocol method.
    fn protocol_receiver_adjustment(
        &mut self,
        base_ty: TyId,
        self_ty: TyId,
        receiver: ReceiverKind,
        span: Span,
    ) -> Option<(ReceiverAdjust, usize)> {
        let expected = match receiver {
            ReceiverKind::Value => self_ty,
            ReceiverKind::Shared => self.ref_ty(RefKind::Shared, self_ty),
            ReceiverKind::Exclusive => self.ref_ty(RefKind::Exclusive, self_ty),
        };
        let exclusive = self.ref_ty(RefKind::Exclusive, self_ty);
        let mut current = base_ty;
        let mut derefs = 0;
        loop {
            if current == expected {
                return Some((ReceiverAdjust::None, derefs));
            }
            if current == self_ty {
                return Some((
                    match receiver {
                        ReceiverKind::Value => ReceiverAdjust::None,
                        ReceiverKind::Shared => ReceiverAdjust::SharedBorrow,
                        ReceiverKind::Exclusive => ReceiverAdjust::ExclusiveBorrow,
                    },
                    derefs,
                ));
            }
            if receiver == ReceiverKind::Shared && current == exclusive {
                return Some((ReceiverAdjust::SharedReborrow, derefs));
            }
            let TyKind::Ref { pointee, .. } = self.hir.ty_ctxt().kind(current) else {
                break;
            };
            current = pointee;
            derefs += 1;
        }
        self.error(
            codes::E3031,
            span,
            format!(
                "indexing cannot pass a value of type `{}` as `{}`",
                self.ty_text(base_ty),
                self.ty_text(expected)
            ),
        )
        .help("dereference the base explicitly, or implement the protocol for its value type")
        .emit();
        None
    }

    fn report_cannot_index(&self, base_ty: TyId, span: Span) {
        self.error(
            codes::E3021,
            span,
            format!("cannot index a value of type `{}`", self.ty_text(base_ty)),
        )
        .help("only arrays, slices, and types implementing `Index` can be indexed")
        .emit();
    }

    fn report_index_not_mutable(&self, base_ty: TyId, span: Span) {
        self.error(
            codes::E3054,
            span,
            format!(
                "cannot write through an index of type `{}`",
                self.ty_text(base_ty)
            ),
        )
        .help("implement `IndexMut` for the base type, or use a read-only expression")
        .emit();
    }

    fn report_ambiguous_index(
        &self,
        protocol: Protocol,
        base_ty: TyId,
        span: Span,
        candidates: &[ProtocolCandidate],
    ) {
        let listed = candidates
            .iter()
            .map(|candidate| self.candidate_text(candidate))
            .collect::<Vec<_>>()
            .join(", ");
        self.error(
            codes::E3053,
            span,
            format!(
                "cannot choose an `{}` implementation for `{}`",
                protocol.name(),
                self.ty_text(base_ty)
            ),
        )
        .help(format!(
            "the candidates are {listed}; use an index expression whose type selects one"
        ))
        .emit();
    }

    pub(super) fn candidate_text(&self, candidate: &ProtocolCandidate) -> String {
        let args = self.candidate_trait_args(candidate).unwrap_or_default();
        let args = args
            .iter()
            .map(|argument| self.ty_text(*argument))
            .collect::<Vec<_>>()
            .join(", ");
        let name = self.hir.definitions().name(candidate.trait_ref.trait_def);
        format!(
            "`{}<{}> for {}`",
            self.interner.resolve(name),
            args,
            self.ty_text(candidate.self_ty)
        )
    }

    fn report_unresolved_index(
        &self,
        protocol: Protocol,
        base_ty: TyId,
        span: Span,
        index_ty: TyId,
        element_ty: TyId,
    ) {
        let mut names = Vec::new();
        if self.contains_infer(index_ty) {
            names.push("`I`");
        }
        if self.contains_infer(element_ty) {
            names.push("`T`");
        }
        let names = names.join(" and ");
        self.error(
            codes::E3055,
            span,
            format!(
                "cannot infer the type argument {names} of `{}` for `{}`",
                protocol.name(),
                self.ty_text(base_ty)
            ),
        )
        .help("annotate the indexed value, or use an impl that fixes every protocol type argument")
        .emit();
    }
}

fn receiver_kind(candidate: &ProtocolCandidate) -> ReceiverKind {
    match &candidate.target {
        CandidateTarget::Impl { receiver, .. }
        | CandidateTarget::Bound { receiver, .. }
        | CandidateTarget::Dynamic { receiver, .. } => *receiver,
    }
}
