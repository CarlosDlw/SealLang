//! Coherence: two impls may not describe the same instance of a trait.
//!
//! The check runs on the impl headers, before any body or method is validated.
//! Two headers overlap when a substitution exists that makes their self types
//! and trait arguments equal; bounds on the impl's parameters are ignored,
//! because a bound restricts which types can use an impl but does not make two
//! impls disjoint. Ambiguity is rejected here so the solver never has to guess.

use seal_error_codes as codes;
use seal_span::Span;
use seal_ty::{TemplateUnifier, TraitRef, TyId};

use super::Lowering;

/// One impl header considered by the coherence check.
struct Header {
    trait_ref: TraitRef,
    self_ty: TyId,
    span: Span,
}

impl Lowering<'_> {
    /// Rejects pairs of trait impls whose headers unify.
    pub(super) fn validate_coherence(&mut self) {
        let headers: Vec<Header> = self
            .pending_trait_impls
            .iter()
            .map(|implementation| Header {
                trait_ref: implementation.trait_ref,
                self_ty: implementation.self_ty,
                span: implementation.span,
            })
            .collect();
        for right_index in 1..headers.len() {
            for left_index in 0..right_index {
                let left = &headers[left_index];
                let right = &headers[right_index];
                if left.trait_ref.trait_def != right.trait_ref.trait_def {
                    continue;
                }
                let Some(witness) = self.overlapping_header(left, right) else {
                    continue;
                };
                // Identical concrete nominal impls are the `E2021` family and
                // were already reported while their blocks were collected.
                if self.is_concrete_duplicate(left) && self.is_concrete_duplicate(right) {
                    continue;
                }
                self.report_overlap(left, right, witness);
            }
        }
    }

    /// Returns the unifying substitution when the two headers can be equal.
    ///
    /// The returned text is empty when the headers are already identical.
    fn overlapping_header(&self, left: &Header, right: &Header) -> Option<Option<String>> {
        let ctxt = self.hir.ty_ctxt();
        let mut unifier = TemplateUnifier::new(ctxt);
        if !unifier.unify(left.trait_ref.self_ty, right.trait_ref.self_ty) {
            return None;
        }
        let left_args = ctxt.substs_args(left.trait_ref.args);
        let right_args = ctxt.substs_args(right.trait_ref.args);
        if left_args.len() != right_args.len() {
            return None;
        }
        for (left_arg, right_arg) in left_args.into_iter().zip(right_args) {
            if !unifier.unify(left_arg, right_arg) {
                return None;
            }
        }
        let matched = unifier.into_match();
        let mut bindings: Vec<(String, String)> = matched
            .left()
            .iter()
            .map(|(param, ty)| {
                let name = self.interner.resolve(ctxt.param_data(*param).name);
                (name.to_owned(), self.display_ty(*ty))
            })
            .collect();
        if bindings.is_empty() {
            return Some(None);
        }
        bindings.sort();
        let text = bindings
            .into_iter()
            .map(|(name, ty)| format!("`{name}` = `{ty}`"))
            .collect::<Vec<_>>()
            .join(", ");
        Some(Some(text))
    }

    /// Returns `true` for a header with no parameters at all.
    ///
    /// Two identical concrete headers are the `E2021` duplicate family and were
    /// already reported when the block was collected.
    fn is_concrete_duplicate(&self, header: &Header) -> bool {
        if self.hir.ty_ctxt().contains_param(header.self_ty) {
            return false;
        }
        self.hir
            .ty_ctxt()
            .substs_args(header.trait_ref.args)
            .into_iter()
            .all(|argument| !self.hir.ty_ctxt().contains_param(argument))
    }

    fn report_overlap(&mut self, left: &Header, right: &Header, witness: Option<String>) {
        let trait_name = self
            .interner
            .resolve(self.hir.definitions().name(right.trait_ref.trait_def))
            .to_owned();
        let right_ty = self.display_ty(right.self_ty);
        let left_ty = self.display_ty(left.self_ty);
        let message = match witness {
            Some(witness) => format!(
                "the implementation of `{trait_name}` for `{right_ty}` overlaps with an \
                 earlier implementation for `{left_ty}` (both apply when {witness})"
            ),
            None => format!(
                "the implementation of `{trait_name}` for `{right_ty}` overlaps with an \
                 earlier implementation for `{left_ty}`"
            ),
        };
        self.error(codes::E2058, right.span, message)
            .secondary(left.span, "the overlapping implementation")
            .help("remove one of the implementations, or make their headers disjoint")
            .emit();
    }
}
