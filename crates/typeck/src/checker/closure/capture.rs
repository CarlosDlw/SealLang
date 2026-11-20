//! Capture places, environment construction, and closure kind.

use seal_def::DefId;
use seal_error_codes as codes;
use seal_hir::{CaptureDef, CaptureSource, HirExprId};
use seal_span::Span;
use seal_ty::{ClosureKind, InitState, RefKind, TyId};

use super::super::flow::Flow;
use super::super::{Checker, PlaceMode};
use crate::results::{
    CaptureInfo, CaptureStorage, ClosureInfo, PlaceRoot, ResolvedPlace, Signature,
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
    /// Types a use of a captured value.
    pub(in crate::checker) fn check_capture(
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
    pub(in crate::checker) fn check_capture_subplace_move(
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

    /// Checks that the value the closure reads is initialized, and marks the
    /// usage a nested closure makes of its parent's environment.
    pub(super) fn check_capture_initializer(
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
    pub(super) fn finalize_closure(&mut self, closure: DefId) {
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

        let copy = captures.iter().all(|capture| match capture.storage {
            CaptureStorage::Shared => true,
            CaptureStorage::Exclusive => false,
            CaptureStorage::Value => self.is_copy(capture.value_ty),
        });
        self.hir.ty_ctxt().set_closure_copy(definition.adt, copy);

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

        self.propagate_outer_usage(closure, &definition, &captures);
    }

    /// Marks how a nested closure's storage borrows its parent's capture.
    fn propagate_outer_usage(
        &mut self,
        closure: DefId,
        definition: &seal_hir::ClosureDef,
        captures: &[CaptureInfo],
    ) {
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
}
