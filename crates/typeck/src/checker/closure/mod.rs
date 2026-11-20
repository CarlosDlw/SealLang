//! Closure literals: signature inference, capture storage, and callable kind.
//!
//! A closure has no declared signature of its own, so the checker combines the
//! annotations in the literal with the signature the position expects. The body
//! then records how each captured value is used: a shared read keeps the
//! environment a shared borrow, a write or exclusive borrow makes it exclusive,
//! and moving a non-`Copy` value out of a non-`move` closure is rejected.
//!
//! Once the body is checked the usage flags decide the storage of every
//! environment field and the closure kind. The synthetic aggregate's field
//! types are replaced before any layout or `Copy` query sees them, and the
//! final [`ClosureInfo`](crate::results::ClosureInfo) is what call checking and
//! coercion consume.

mod capture;
mod signature;

use seal_def::DefId;
use seal_hir::CaptureSource;
use seal_span::Span;
use seal_ty::{ClosureKind, TyId, TyKind};

use super::Checker;
use super::flow::Flow;
use crate::results::{CaptureInfo, CaptureStorage, ClosureInfo, Signature};

impl Checker<'_> {
    /// Types a closure literal and checks its body.
    ///
    /// The body runs with its own flow state but shares the enclosing body's
    /// inference context, so closure parameters can take part in the same
    /// unification as the values around the literal.
    pub(super) fn check_closure(
        &mut self,
        closure: DefId,
        expected: Option<TyId>,
        flow: &mut Flow,
        span: Span,
    ) -> TyId {
        if self.checked_closures.contains(&closure) {
            return self.closure_ty(closure);
        }
        let Some(definition) = self.hir.closure(closure).cloned() else {
            panic!("closure {closure:?} has no closure data; this is an internal compiler error");
        };
        let Some(body) = self.hir.body_of(closure).cloned() else {
            panic!("closure {closure:?} has no body; this is an internal compiler error");
        };
        self.checked_closures.insert(closure);

        let hint = self.expected_signature(expected);
        let params = self.closure_params(&body, hint.as_ref());
        let (ret, inferred_ret) = self.closure_ret(&body, hint.as_ref(), span);
        self.check_expected_signature(span, hint.as_ref(), &params, ret);

        let mut captures = Vec::with_capacity(definition.captures.len());
        for index in 0..definition.captures.len() {
            let value_ty = self
                .capture_value_ty(closure, index)
                .unwrap_or_else(|| self.error_ty());
            captures.push(CaptureInfo {
                value_ty,
                storage: if definition.is_move {
                    CaptureStorage::Value
                } else {
                    CaptureStorage::Shared
                },
                shared: false,
                exclusive: false,
                moved: false,
                move_reported: false,
            });
        }
        let signature = Signature {
            params: params.clone(),
            ret,
            is_unsafe: false,
            generics: self.hir.flattened_generics(closure),
        };
        self.results.insert_signature(closure, signature.clone());
        self.results.record_closure(
            closure,
            definition.adt,
            ClosureInfo {
                signature,
                kind: ClosureKind::Fn,
                is_move: definition.is_move,
                captures,
            },
        );

        for index in 0..definition.captures.len() {
            let capture = definition.captures[index];
            self.check_capture_initializer(closure, index, capture, flow);
        }

        let saved = (
            self.owner,
            self.current_closure,
            self.ret,
            self.env.clone(),
            self.proof_cache.clone(),
            self.const_fn,
            self.const_initializer,
            self.unsafe_depth,
            self.mut_ref_reported,
            self.guard_moves.take(),
            self.checking_place_base,
        );
        self.owner = Some(closure);
        self.current_closure = Some(closure);
        self.ret = ret;
        self.env = self.hir.param_env(closure);
        self.proof_cache.clear();
        self.const_fn = false;
        self.const_initializer = false;
        self.unsafe_depth = 0;
        self.mut_ref_reported = false;
        self.checking_place_base = false;

        let mut body_flow = Flow::new();
        for (param, ty) in body.params.iter().zip(&params) {
            self.results.record_local(param.def, *ty);
            body_flow.assign(param.def, &[]);
        }
        let outcome = self.check_block(body.root, (!inferred_ret).then_some(ret), body_flow);
        if inferred_ret {
            self.mismatch(span, ret, outcome.ty);
        }

        let (
            owner,
            current_closure,
            ret,
            env,
            proof_cache,
            const_fn,
            const_initializer,
            unsafe_depth,
            mut_ref_reported,
            guard_moves,
            checking_place_base,
        ) = saved;
        self.owner = owner;
        self.current_closure = current_closure;
        self.ret = ret;
        self.env = env;
        self.proof_cache = proof_cache;
        self.const_fn = const_fn;
        self.const_initializer = const_initializer;
        self.unsafe_depth = unsafe_depth;
        self.mut_ref_reported = mut_ref_reported;
        self.guard_moves = guard_moves;
        self.checking_place_base = checking_place_base;

        self.finalize_closure(closure);
        self.closure_ty(closure)
    }

    /// Returns the closure definition named by `ty`, if it is a closure value.
    pub(super) fn closure_of_ty(&self, ty: TyId) -> Option<DefId> {
        let TyKind::Adt { adt, .. } = self.hir.ty_ctxt().kind(ty) else {
            return None;
        };
        self.hir
            .closure_for_adt(adt)
            .or_else(|| self.results.closure_adt_owner(adt))
    }

    /// Returns the type of a closure value, with the enclosing generics as
    /// arguments of its synthetic aggregate.
    fn closure_ty(&self, closure: DefId) -> TyId {
        let Some(definition) = self.hir.closure(closure) else {
            return self.error_ty();
        };
        let args: Vec<TyId> = self
            .hir
            .flattened_generics(closure)
            .iter()
            .map(|param| self.interned(TyKind::Param(param.param)))
            .collect();
        let args = self.hir.ty_ctxt().substs(&args);
        self.hir.ty_ctxt().adt_ty_with(definition.adt, args)
    }

    /// Returns the type captured at `index` in `closure`, before any borrow is
    /// taken. A nested closure reads the value through its parent's environment.
    pub(super) fn capture_value_ty(&self, closure: DefId, index: usize) -> Option<TyId> {
        let capture = self.hir.closure(closure)?.captures.get(index)?;
        match capture.source {
            CaptureSource::Local(origin) => Some(self.local_type(origin)),
            CaptureSource::Outer(slot) => {
                let parent = self.hir.definitions().get(closure).parent?;
                self.results
                    .closure_info(parent)?
                    .captures
                    .get(slot as usize)
                    .map(|capture| capture.value_ty)
            }
        }
    }
}
