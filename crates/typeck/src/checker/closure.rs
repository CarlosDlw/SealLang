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
//! final [`ClosureInfo`] is what call checking and coercion consume.

use seal_def::DefId;
use seal_error_codes as codes;
use seal_hir::{Body, CaptureDef, CaptureSource, HirExprId};
use seal_span::Span;
use seal_ty::{ClosureKind, InitState, RefKind, TyId, TyKind};

use super::flow::Flow;
use super::{Checker, PlaceMode};
use crate::results::{
    CaptureInfo, CaptureStorage, ClosureHint, ClosureInfo, PlaceRoot, ResolvedPlace, Signature,
};

/// How the body used one captured value.
#[derive(Clone, Copy)]
enum CaptureUsage {
    /// The value is read without writing it.
    Shared,
    /// The value is written or exclusively borrowed.
    Exclusive,
    /// The value is moved out of the environment.
    Moved,
}

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

    /// Types a use of a captured value.
    pub(super) fn check_capture(
        &mut self,
        expr: HirExprId,
        closure: DefId,
        index: u32,
        mode: PlaceMode,
        span: Span,
    ) -> TyId {
        if self.current_closure != Some(closure) {
            panic!(
                "capture of {closure:?} used outside its body; \
                 this is an internal compiler error"
            );
        }
        let Some(is_move) = self
            .hir
            .closure(closure)
            .map(|definition| definition.is_move)
        else {
            return self.error_ty();
        };
        let Some(capture) = self
            .hir
            .closure(closure)
            .and_then(|definition| definition.captures.get(index as usize).copied())
        else {
            let name = self.def_name(closure).to_owned();
            self.error(
                codes::E3095,
                span,
                format!("the closure `{name}` has no capture at slot {index}"),
            )
            .note("this is a compiler bug in closure lowering")
            .emit();
            return self.error_ty();
        };
        let Some(value_ty) = self
            .results
            .closure_info(closure)
            .and_then(|info| info.captures.get(index as usize))
            .map(|capture| capture.value_ty)
        else {
            panic!(
                "closure {closure:?} has no checked capture {index}; \
                 this is an internal compiler error"
            );
        };

        self.results.record_place(
            expr,
            ResolvedPlace {
                root: PlaceRoot::Capture {
                    closure,
                    index,
                    binding: capture.binding,
                },
                root_ty: value_ty,
                projections: Vec::new(),
                ty: value_ty,
                deref: None,
                raw_deref: None,
            },
        );

        match mode {
            PlaceMode::Write | PlaceMode::BorrowExclusive => {
                self.note_capture(closure, index, CaptureUsage::Exclusive, value_ty);
            }
            PlaceMode::Borrow => {
                self.note_capture(closure, index, CaptureUsage::Shared, value_ty);
            }
            PlaceMode::Read => {
                if self.checking_place_base {
                    // Only a sub-place is selected; moving a non-`Copy` field
                    // is rejected by the projection check.
                } else if self.is_copy(value_ty) || self.is_error(value_ty) {
                    self.note_capture(closure, index, CaptureUsage::Shared, value_ty);
                } else if is_move {
                    self.note_capture(closure, index, CaptureUsage::Moved, value_ty);
                } else {
                    self.report_capture_move(closure, index, span);
                }
            }
        }
        value_ty
    }

    /// Reports a read of a non-`Copy` field or element that would move it out
    /// of a captured reference.
    pub(super) fn check_capture_subplace_move(
        &mut self,
        base: HirExprId,
        selected: TyId,
        mode: PlaceMode,
        span: Span,
    ) {
        if mode != PlaceMode::Read || self.is_copy(selected) || self.is_error(selected) {
            return;
        }
        let Some(place) = self.results.place(base).cloned() else {
            return;
        };
        let PlaceRoot::Capture { closure, index, .. } = place.root else {
            return;
        };
        let is_move = self
            .hir
            .closure(closure)
            .is_some_and(|definition| definition.is_move);
        let Some(value_ty) = self
            .results
            .closure_info(closure)
            .and_then(|info| info.captures.get(index as usize))
            .map(|capture| capture.value_ty)
        else {
            return;
        };
        if is_move {
            self.note_capture(closure, index, CaptureUsage::Moved, value_ty);
            return;
        }
        self.report_capture_move(closure, index, span);
    }

    /// Returns the expected callable signature at a closure literal.
    pub(super) fn expected_signature(&mut self, expected: Option<TyId>) -> Option<ClosureHint> {
        let expected = self.resolve_deep(expected?);
        let parts = self.hir.ty_ctxt().fn_ptr_parts(expected)?;
        Some(ClosureHint {
            params: parts.params,
            ret: parts.ret,
        })
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
    pub(super) fn closure_ty(&self, closure: DefId) -> TyId {
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

    fn closure_params(&mut self, body: &Body, hint: Option<&ClosureHint>) -> Vec<TyId> {
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
    fn closure_ret(&mut self, body: &Body, hint: Option<&ClosureHint>, span: Span) -> (TyId, bool) {
        let declared = body.ret.filter(|ty| !self.is_error(*ty));
        match (declared, hint.map(|hint| hint.ret)) {
            (Some(declared), _) => (self.storage_ty(declared, span, "closure result"), false),
            (None, Some(expected)) => (expected, false),
            (None, None) => (self.infer.fresh_ty_var(self.hir.ty_ctxt()), true),
        }
    }

    /// Compares the closure signature with the expected one and reports one
    /// diagnostic carrying both signatures.
    fn check_expected_signature(
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

    fn check_capture_initializer(
        &mut self,
        closure: DefId,
        index: usize,
        capture: CaptureDef,
        flow: &Flow,
    ) {
        let Some(value_ty) = self.capture_value_ty(closure, index) else {
            return;
        };
        let span = self.hir.definitions().get(capture.binding).span;
        match capture.source {
            CaptureSource::Local(origin) => {
                let place = ResolvedPlace {
                    root: PlaceRoot::Local(origin),
                    root_ty: value_ty,
                    projections: Vec::new(),
                    ty: value_ty,
                    deref: None,
                    raw_deref: None,
                };
                if self.is_error(value_ty)
                    || self.place_state(&place, flow) == InitState::Initialized
                {
                    return;
                }
                let text = self.def_name(origin).to_owned();
                self.error(
                    codes::E3011,
                    span,
                    format!("`{text}` is possibly uninitialized"),
                )
                .help(format!(
                    "assign a value to `{text}` before the closure captures it"
                ))
                .emit();
            }
            CaptureSource::Outer(slot) => {
                let Some(parent) = self.hir.definitions().get(closure).parent else {
                    return;
                };
                let Some(parent_is_move) = self
                    .hir
                    .closure(parent)
                    .map(|definition| definition.is_move)
                else {
                    return;
                };
                let moved = self
                    .hir
                    .closure(closure)
                    .is_some_and(|definition| definition.is_move);
                if moved && !self.is_copy(value_ty) {
                    if parent_is_move {
                        self.note_capture(parent, slot, CaptureUsage::Moved, value_ty);
                    } else {
                        self.report_capture_move_in(closure, index as u32, span);
                    }
                } else {
                    self.note_capture(parent, slot, CaptureUsage::Shared, value_ty);
                }
            }
        }
    }

    /// Converts the usage flags of `closure` into storage, fields, and kind.
    fn finalize_closure(&mut self, closure: DefId) {
        let Some(definition) = self.hir.closure(closure).cloned() else {
            return;
        };
        let Some(info) = self.results.closure_info(closure).cloned() else {
            return;
        };
        let params: Vec<TyId> = info
            .signature
            .params
            .iter()
            .map(|ty| self.resolve_deep(*ty))
            .collect();
        let mut ret = self.resolve_deep(info.signature.ret);
        if self.contains_infer(ret) {
            ret = self.error_ty();
        }

        if !definition.is_move {
            for (index, capture_def) in definition.captures.iter().enumerate() {
                let CaptureSource::Local(origin) = capture_def.source else {
                    continue;
                };
                if !info
                    .captures
                    .get(index)
                    .is_some_and(|capture| capture.exclusive)
                {
                    continue;
                }
                if self.root_is_writable(PlaceRoot::Local(origin)) {
                    continue;
                }
                let name = self.def_name(origin).to_owned();
                let span = self.hir.definitions().get(capture_def.binding).span;
                self.error(
                    codes::E3017,
                    span,
                    format!("cannot capture `{name}` as mutable, as it is not declared as mutable"),
                )
                .help(format!("declare `{name}` with `let mut` to allow mutation"))
                .emit();
            }
        }

        let mut kind = ClosureKind::Fn;
        let mut captures = Vec::with_capacity(info.captures.len());
        for capture in &info.captures {
            let value_ty = self.resolve_deep(capture.value_ty);
            let storage = if definition.is_move {
                CaptureStorage::Value
            } else if capture.exclusive {
                CaptureStorage::Exclusive
            } else {
                CaptureStorage::Shared
            };
            if capture.moved {
                kind = ClosureKind::FnOnce;
            } else if capture.exclusive && kind == ClosureKind::Fn {
                kind = ClosureKind::FnMut;
            }
            captures.push(CaptureInfo {
                value_ty,
                storage,
                shared: capture.shared,
                exclusive: capture.exclusive,
                moved: capture.moved,
                move_reported: capture.move_reported,
            });
        }

        for (index, capture) in captures.iter().enumerate() {
            let field_ty = match capture.storage {
                CaptureStorage::Value => capture.value_ty,
                CaptureStorage::Shared => self.ref_ty(RefKind::Shared, capture.value_ty),
                CaptureStorage::Exclusive => self.ref_ty(RefKind::Exclusive, capture.value_ty),
            };
            self.hir
                .ty_ctxt()
                .set_field_ty(definition.adt, index, field_ty);
        }

        let signature = Signature {
            params,
            ret,
            is_unsafe: false,
            generics: info.signature.generics,
        };
        self.results.insert_signature(closure, signature.clone());
        self.results.record_closure(
            closure,
            definition.adt,
            ClosureInfo {
                signature,
                kind,
                is_move: definition.is_move,
                captures: captures.clone(),
            },
        );

        for (index, capture_def) in definition.captures.iter().enumerate() {
            let CaptureSource::Outer(slot) = capture_def.source else {
                continue;
            };
            let Some(parent) = self.hir.definitions().get(closure).parent else {
                continue;
            };
            let capture = captures[index];
            match capture.storage {
                CaptureStorage::Value if !self.is_copy(capture.value_ty) => {
                    self.note_capture(parent, slot, CaptureUsage::Moved, capture.value_ty);
                }
                CaptureStorage::Value | CaptureStorage::Shared => {
                    self.note_capture(parent, slot, CaptureUsage::Shared, capture.value_ty);
                }
                CaptureStorage::Exclusive => {
                    self.note_capture(parent, slot, CaptureUsage::Exclusive, capture.value_ty);
                }
            }
        }
    }

    /// Records one usage of a capture, copying `Copy` values instead of moving
    /// them.
    fn note_capture(&mut self, closure: DefId, index: u32, usage: CaptureUsage, value_ty: TyId) {
        let copied = self.is_copy(value_ty);
        let Some(info) = self.results.closure_info_mut(closure) else {
            return;
        };
        let Some(capture) = info.captures.get_mut(index as usize) else {
            return;
        };
        match usage {
            CaptureUsage::Shared => capture.shared = true,
            CaptureUsage::Exclusive => capture.exclusive = true,
            CaptureUsage::Moved if copied => capture.shared = true,
            CaptureUsage::Moved => capture.moved = true,
        }
    }

    fn report_capture_move(&mut self, closure: DefId, index: u32, span: Span) {
        let Some(info) = self.results.closure_info_mut(closure) else {
            return;
        };
        let Some(capture) = info.captures.get_mut(index as usize) else {
            return;
        };
        if capture.move_reported {
            return;
        }
        capture.move_reported = true;
        self.report_capture_move_in(closure, index, span);
    }

    fn report_capture_move_in(&mut self, closure: DefId, index: u32, span: Span) {
        let Some(name) = self
            .hir
            .closure(closure)
            .and_then(|definition| definition.captures.get(index as usize))
            .map(|capture| self.def_name(capture.binding).to_owned())
        else {
            return;
        };
        self.error(
            codes::E3095,
            span,
            format!("cannot move the captured value `{name}` out of a borrowed environment"),
        )
        .help(
            "add `move` before the closure parameters so the closure owns the value, or borrow it instead",
        )
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

    fn capture_value_ty(&self, closure: DefId, index: usize) -> Option<TyId> {
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
