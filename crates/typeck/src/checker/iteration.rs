//! Custom iteration protocol resolution.
//!
//! A `for` source that is not a built-in range or sequence resolves through
//! `IntoIterator` first and falls back to treating the source as an `Iterator`.
//! Candidate calls are built after the loop body, because the body may be the
//! only place that fixes the item type.

use seal_error_codes as codes;
use seal_hir::{HirExprId, HirPatternId, PatternKind, Protocol, ProtocolMethod};
use seal_span::Span;
use seal_ty::TyId;

use super::Checker;
use super::protocol::ProtocolCandidate;
use crate::results::{IterationProtocol, ProtocolCall};

/// The result of looking for an `Iterator` implementation.
enum IteratorSelection {
    /// Exactly one matching implementation.
    Found(ProtocolCandidate),
    /// No implementation at any reference layer.
    Missing,
    /// More than one implementation at the winning layer.
    Ambiguous,
}

/// A resolved iteration plan whose calls are not built yet.
///
/// The loop body can constrain the item type, so building calls here would
/// reject a generic impl parameter before that constraint is available.
pub(super) struct PendingIteration {
    pub(super) item_ty: TyId,
    pub(super) iterator_ty: TyId,
    pub(super) into_candidate: Option<ProtocolCandidate>,
    pub(super) iterator_candidate: ProtocolCandidate,
    pub(super) direct_iterator: bool,
}

impl Checker<'_> {
    /// Resolves the custom protocol plan for one `for` source.
    ///
    /// Returns `None` after reporting an error. The caller checks the body and
    /// then calls [`Self::finalize_iteration`], which resolves the item type and
    /// builds the recorded calls.
    pub(super) fn resolve_iteration(
        &mut self,
        _source: HirExprId,
        source_ty: TyId,
        span: Span,
    ) -> Option<PendingIteration> {
        if self.is_error(source_ty) {
            return None;
        }
        let resolved_source_ty = self.resolve_deep(source_ty);
        if self.contains_infer(resolved_source_ty) {
            self.report_unresolved_iteration(source_ty, false, span);
            return None;
        }

        let into_trait = self.hir.protocol_trait(Protocol::IntoIterator);
        let iter_method = self
            .hir
            .protocol_defs()
            .method(Protocol::IntoIterator, ProtocolMethod::Iter);
        let iter_name = self.hir.definitions().name(iter_method);
        let into_candidates = self.protocol_candidates(source_ty, into_trait, iter_name, false);

        match into_candidates.len() {
            0 => self.resolve_direct_iterator(source_ty, span),
            1 => {
                let into_candidate = into_candidates
                    .into_iter()
                    .next()
                    .expect("the candidate count was one");
                self.resolve_into_iterator(source_ty, into_candidate, span)
            }
            _ => {
                self.report_ambiguous_iteration(
                    Protocol::IntoIterator,
                    source_ty,
                    span,
                    &into_candidates,
                );
                None
            }
        }
    }

    /// Finalizes an item type that the loop body may have inferred.
    ///
    /// The pattern is already bound, so a direct binding is refreshed to the
    /// concrete item type. Calls are built only now, after `I` and `T` are
    /// known, and the protocol is recorded only on success.
    pub(super) fn finalize_iteration(
        &mut self,
        source: HirExprId,
        pattern: HirPatternId,
        source_ty: TyId,
        pending: PendingIteration,
        span: Span,
    ) -> Option<TyId> {
        let item_ty = self.resolve_deep(pending.item_ty);
        if self.is_error(item_ty) {
            return None;
        }
        if self.contains_infer(item_ty) {
            self.report_unresolved_iteration(source_ty, false, span);
            return None;
        }
        let iterator_ty = self.resolve_deep(pending.iterator_ty);
        if self.contains_infer(iterator_ty) {
            self.report_unresolved_iteration(source_ty, true, span);
            return None;
        }

        let iter = match &pending.into_candidate {
            Some(candidate) => Some(self.build_protocol_call(candidate, source_ty, span)?),
            None => None,
        };
        let has_next = self.build_iterator_call(
            &pending.iterator_candidate,
            iterator_ty,
            ProtocolMethod::HasNext,
            span,
        )?;
        let next = self.build_iterator_call(
            &pending.iterator_candidate,
            iterator_ty,
            ProtocolMethod::Next,
            span,
        )?;
        let iterator_ty = iter.as_ref().map_or(iterator_ty, |call| call.method.ret_ty);

        self.refresh_direct_bindings(pattern, item_ty);
        let protocol = IterationProtocol {
            item_ty,
            iterator_ty,
            iter,
            has_next,
            next,
            direct_iterator: pending.direct_iterator,
        };
        self.results.record_iteration(source, protocol);
        self.check_const_iteration(source, span);
        Some(item_ty)
    }

    /// Updates direct bindings after an inferred item type becomes concrete.
    ///
    /// A destructuring pattern over an unresolved item type cannot be checked
    /// until the type is known, so only whole-value bindings can be finalized
    /// here.
    fn refresh_direct_bindings(&mut self, pattern: HirPatternId, ty: TyId) {
        let kind = self.hir.pattern(pattern).kind.clone();
        if let PatternKind::Binding {
            def,
            pattern: subpattern,
            ..
        } = kind
        {
            self.results.record_local(def, ty);
            if let Some(subpattern) = subpattern {
                self.refresh_direct_bindings(subpattern, ty);
            }
        }
    }

    /// Resolves the direct-`Iterator` fallback for a source without
    /// `IntoIterator`.
    fn resolve_direct_iterator(&mut self, source_ty: TyId, span: Span) -> Option<PendingIteration> {
        let iterator_candidate = match self.select_iterator(source_ty, None, span) {
            IteratorSelection::Found(candidate) => candidate,
            IteratorSelection::Missing => {
                self.report_not_iterable(source_ty, span);
                return None;
            }
            IteratorSelection::Ambiguous => return None,
        };
        let item_ty = self.candidate_item_ty(&iterator_candidate)?;

        Some(PendingIteration {
            item_ty,
            iterator_ty: source_ty,
            into_candidate: None,
            iterator_candidate,
            direct_iterator: true,
        })
    }

    /// Resolves an `IntoIterator` source and its resulting iterator.
    fn resolve_into_iterator(
        &mut self,
        source_ty: TyId,
        into_candidate: ProtocolCandidate,
        span: Span,
    ) -> Option<PendingIteration> {
        let args = self.candidate_trait_args(&into_candidate)?;
        if args.len() != 2 {
            return None;
        }
        let iterator_ty = self.resolve_deep(args[0]);
        let item_ty = args[1];

        let iterator_candidate = match self.select_iterator(iterator_ty, Some(item_ty), span) {
            IteratorSelection::Found(candidate) => candidate,
            IteratorSelection::Missing => {
                if self.contains_infer(iterator_ty) {
                    self.report_unresolved_iteration(source_ty, true, span);
                } else {
                    self.report_missing_iterator(iterator_ty, item_ty, span);
                }
                return None;
            }
            IteratorSelection::Ambiguous => return None,
        };
        let item_ty = self.candidate_item_ty(&iterator_candidate)?;

        Some(PendingIteration {
            item_ty,
            iterator_ty,
            into_candidate: Some(into_candidate),
            iterator_candidate,
            direct_iterator: false,
        })
    }

    /// Selects exactly one `Iterator` implementation, peeling reference layers.
    ///
    /// When `expected_item` is present, a candidate whose item type cannot
    /// unify with it is skipped. The caller reports `E3051` when no layer
    /// provides a matching implementation.
    fn select_iterator(
        &mut self,
        ty: TyId,
        expected_item: Option<TyId>,
        span: Span,
    ) -> IteratorSelection {
        let iterator_trait = self.hir.protocol_trait(Protocol::Iterator);
        let next_method = self
            .hir
            .protocol_defs()
            .method(Protocol::Iterator, ProtocolMethod::Next);
        let next_name = self.hir.definitions().name(next_method);

        for layer_ty in self.reference_layers(ty) {
            let candidates = self.protocol_candidates(layer_ty, iterator_trait, next_name, false);
            match candidates.len() {
                0 => continue,
                1 => {
                    let candidate = candidates
                        .into_iter()
                        .next()
                        .expect("the candidate count was one");
                    if let Some(expected_item) = expected_item {
                        let Some(candidate_item) = self.candidate_item_ty(&candidate) else {
                            continue;
                        };
                        let mut trial = self.infer.clone();
                        if trial
                            .unify(self.hir.ty_ctxt(), candidate_item, expected_item)
                            .is_err()
                        {
                            continue;
                        }
                        let _ = self
                            .infer
                            .unify(self.hir.ty_ctxt(), candidate_item, expected_item);
                    }
                    return IteratorSelection::Found(candidate);
                }
                _ => {
                    self.report_ambiguous_iteration(
                        Protocol::Iterator,
                        layer_ty,
                        span,
                        &candidates,
                    );
                    return IteratorSelection::Ambiguous;
                }
            }
        }
        IteratorSelection::Missing
    }

    /// Builds one `Iterator` call for an already selected iterator.
    fn build_iterator_call(
        &mut self,
        candidate: &ProtocolCandidate,
        receiver_ty: TyId,
        method: ProtocolMethod,
        span: Span,
    ) -> Option<ProtocolCall> {
        let method_def = self.hir.protocol_defs().method(Protocol::Iterator, method);
        let method_name = self.hir.definitions().name(method_def);
        let candidate = self.with_protocol_method(candidate, method_name)?;
        self.build_protocol_call(&candidate, receiver_ty, span)
    }

    /// Returns the item type declared by a selected iterator candidate.
    fn candidate_item_ty(&self, candidate: &ProtocolCandidate) -> Option<TyId> {
        let args = self.candidate_trait_args(candidate)?;
        args.first().copied()
    }

    fn report_not_iterable(&self, source_ty: TyId, span: Span) {
        self.error(
            codes::E3035,
            span,
            format!("cannot iterate over `{}`", self.ty_text(source_ty)),
        )
        .help(
            "implement `IntoIterator` for the source type, or `Iterator` directly, \
             or iterate an integer range, an array, or a slice reference",
        )
        .emit();
    }

    fn report_ambiguous_iteration(
        &self,
        protocol: Protocol,
        source_ty: TyId,
        span: Span,
        candidates: &[ProtocolCandidate],
    ) {
        let listed = candidates
            .iter()
            .map(|candidate| self.candidate_text(candidate))
            .collect::<Vec<_>>()
            .join(", ");
        let code = if protocol == Protocol::IntoIterator {
            codes::E3050
        } else {
            codes::E3052
        };
        self.error(
            code,
            span,
            format!(
                "cannot choose an `{}` implementation for `{}`",
                protocol.name(),
                self.ty_text(source_ty)
            ),
        )
        .help(format!(
            "the candidates are {listed}; use a newtype with one implementation, \
             or make the source type select exactly one"
        ))
        .emit();
    }

    fn report_missing_iterator(&self, iterator_ty: TyId, item_ty: TyId, span: Span) {
        self.error(
            codes::E3051,
            span,
            format!(
                "the iterator type `{}` does not implement `Iterator<{}>`",
                self.ty_text(iterator_ty),
                self.ty_text(item_ty)
            ),
        )
        .help("implement `Iterator` for the iterator type with that item type")
        .emit();
    }

    fn report_unresolved_iteration(&self, source_ty: TyId, iterator_type: bool, span: Span) {
        let parameter = if iterator_type {
            "iterator type `I`"
        } else {
            "source type"
        };
        self.error(
            codes::E3055,
            span,
            format!(
                "cannot infer the {parameter} of the iteration protocol for `{}`",
                self.ty_text(source_ty)
            ),
        )
        .help("annotate the source or the item binding to fix the protocol type arguments")
        .emit();
    }
}
