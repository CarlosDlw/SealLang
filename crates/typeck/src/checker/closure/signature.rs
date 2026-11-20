//! Closure signatures: annotations, the expected callable, and inference.

use seal_error_codes as codes;
use seal_hir::Body;
use seal_span::Span;
use seal_ty::TyId;

use super::super::Checker;
use crate::results::ClosureHint;

impl Checker<'_> {
    /// Returns the expected callable signature at a closure literal.
    ///
    /// Only a function pointer fixes both lists in this phase. A concrete
    /// `Fn` bound supplies the same information once the built-in traits exist.
    pub(super) fn expected_signature(&mut self, expected: Option<TyId>) -> Option<ClosureHint> {
        let expected = self.resolve_deep(expected?);
        let parts = self.hir.ty_ctxt().fn_ptr_parts(expected)?;
        Some(ClosureHint {
            params: parts.params,
            ret: parts.ret,
        })
    }

    /// Resolves the parameter types from annotations, falling back to the
    /// expected signature.
    pub(super) fn closure_params(&mut self, body: &Body, hint: Option<&ClosureHint>) -> Vec<TyId> {
        let mut params = Vec::with_capacity(body.params.len());
        for (index, param) in body.params.iter().enumerate() {
            let span = self.hir.definitions().get(param.def).span;
            let ty = if self.is_error(param.ty) {
                match hint.and_then(|hint| hint.params.get(index).copied()) {
                    Some(hinted) => self.storage_ty(hinted, span, "closure parameter"),
                    None => {
                        let name = self.def_name(param.def).to_owned();
                        self.error(
                            codes::E3094,
                            span,
                            format!("cannot infer the type of the closure parameter `{name}`"),
                        )
                        .help(
                            "annotate the parameter, or use the closure where a `func(...)` type or an `Fn` bound supplies the signature",
                        )
                        .emit();
                        self.error_ty()
                    }
                }
            } else {
                self.storage_ty(param.ty, span, "closure parameter")
            };
            params.push(ty);
        }
        params
    }

    /// Returns the closure result type and whether it must still be inferred
    /// from the body.
    pub(super) fn closure_ret(
        &mut self,
        body: &Body,
        hint: Option<&ClosureHint>,
        span: Span,
    ) -> (TyId, bool) {
        let declared = body.ret.filter(|ty| !self.is_error(*ty));
        match (declared, hint.map(|hint| hint.ret)) {
            (Some(declared), _) => (self.storage_ty(declared, span, "closure result"), false),
            (None, Some(expected)) => (expected, false),
            (None, None) => (self.infer.fresh_ty_var(self.hir.ty_ctxt()), true),
        }
    }

    /// Compares the closure signature with the expected one and reports one
    /// diagnostic carrying both signatures.
    pub(super) fn check_expected_signature(
        &mut self,
        span: Span,
        hint: Option<&ClosureHint>,
        params: &[TyId],
        ret: TyId,
    ) {
        let Some(hint) = hint else {
            return;
        };
        let mut matches = hint.params.len() == params.len();
        if matches {
            for (expected, found) in hint.params.iter().zip(params) {
                if !self.is_error(*found) && !self.is_error(*expected) && expected != found {
                    matches = false;
                    break;
                }
            }
        }
        let ret_matches = self.is_error(ret) || self.is_error(hint.ret) || hint.ret == ret;
        if matches && ret_matches {
            return;
        }
        let expected = self.signature_text(&hint.params, hint.ret);
        let found = self.signature_text(params, ret);
        self.error(
            codes::E3094,
            span,
            format!("closure signature `{found}` does not match the expected `{expected}`"),
        )
        .help("annotate the closure to match the expected signature")
        .emit();
    }

    fn signature_text(&self, params: &[TyId], ret: TyId) -> String {
        let params = params
            .iter()
            .map(|ty| {
                if self.is_error(*ty) {
                    "_".to_owned()
                } else {
                    self.ty_text(*ty)
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        let ret = if self.is_error(ret) {
            "_".to_owned()
        } else {
            self.ty_text(ret)
        };
        format!("func({params}) -> {ret}")
    }
}
