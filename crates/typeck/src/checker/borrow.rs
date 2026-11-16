//! Reference types: borrowing places and reading through references.

use seal_error_codes as codes;
use seal_hir::{ExprKind, HirExprId};
use seal_span::Span;
use seal_ty::{RefKind, TyId, TyKind};

use super::flow::Flow;
use super::{Checker, PlaceMode};

impl Checker<'_> {
    /// Types `&place` as `*T` and `&mut place` as `*mut T`, where `T` is the
    /// type of the borrowed place.
    ///
    /// A place is a binding, a parameter, a field, or a dereference of a
    /// reference — including a reference value such as a call result, because
    /// `*reference` names the pointee wherever the reference came from. A
    /// computed value has no storage, and Seal does not create hidden
    /// temporaries for borrows. Reading the borrowed place also requires it to
    /// be initialized, which the borrow mode checks.
    ///
    /// `&mut place` additionally requires the place to permit writes: a binding
    /// declared `mut`, or a place whose last dereference is `*mut T`.
    pub(super) fn check_borrow(
        &mut self,
        operand: HirExprId,
        kind: RefKind,
        flow: &mut Flow,
    ) -> TyId {
        let is_place = self.hir.place(operand).is_some()
            || matches!(self.hir.expr(operand).kind, ExprKind::Deref { .. });
        let ty = self.infer_expr_mode(operand, None, flow, PlaceMode::Borrow);

        if !is_place {
            if !self.is_error(ty) {
                let span = self.hir.expr(operand).span;
                self.error(codes::E3013, span, "cannot borrow this expression")
                    .help("borrow a place, such as a binding or a parameter")
                    .emit();
            }
            return self.error_ty();
        }

        if self.is_error(ty) {
            return ty;
        }
        if kind.is_exclusive() && !self.borrow_is_writable(operand) {
            self.report_mutable_borrow(operand);
            return self.error_ty();
        }
        self.ref_ty(kind, ty)
    }

    /// Returns `true` when `operand` names a place an exclusive borrow may
    /// write.
    fn borrow_is_writable(&self, operand: HirExprId) -> bool {
        match self.deref_kind(operand) {
            Some(kind) => kind.is_exclusive(),
            None => self
                .hir
                .place(operand)
                .is_some_and(|place| self.hir.definitions().is_mutable(place.local_def())),
        }
    }

    /// Returns the kind of the last reference `expr` reads through, if any.
    ///
    /// A place carries the kind recorded when it was resolved. A dereference of
    /// a reference *value*, such as `*identity(p)`, has no resolved place, so
    /// the kind comes from the type of the dereferenced expression.
    pub(super) fn deref_kind(&self, expr: HirExprId) -> Option<RefKind> {
        match self.results.place(expr) {
            Some(place) if place.contains_deref() => place.deref,
            Some(_) => None,
            None => match self.hir.expr(expr).kind {
                ExprKind::Deref { operand } => self.reference_kind(operand),
                _ => None,
            },
        }
    }

    /// Returns the kind of the reference type of `expr`, if it is one.
    pub(super) fn reference_kind(&self, expr: HirExprId) -> Option<RefKind> {
        let ty = self.results.expr_ty(expr)?;
        self.hir.ty_ctxt().ref_parts(ty).map(|(kind, _)| kind)
    }

    fn report_mutable_borrow(&self, operand: HirExprId) {
        if self.deref_kind(operand).is_some()
            || self
                .results
                .place(operand)
                .is_some_and(|place| place.contains_deref())
        {
            self.error(
                codes::E3017,
                self.hir.expr(operand).span,
                "cannot borrow through a shared reference as mutable",
            )
            .help("`*T` is read-only; use a `*mut T` reference to write through it")
            .emit();
            return;
        }
        let text = self
            .results
            .place(operand)
            .map_or_else(|| "this place".to_owned(), |place| self.place_text(place));
        self.error(
            codes::E3017,
            self.hir.expr(operand).span,
            format!("cannot borrow `{text}` as mutable, as it is not declared as mutable"),
        )
        .help(format!("declare `{text}` with `let mut` to allow mutation"))
        .emit();
    }

    /// Types `*reference` as the pointee of its reference type.
    ///
    /// The pointee may be unsized (`str`); whether it can be stored is decided
    /// by the context that uses the dereference.
    pub(super) fn check_deref(&mut self, operand: HirExprId, flow: &mut Flow, span: Span) -> TyId {
        let pointer = self.infer_expr(operand, None, flow);
        match self.hir.ty_ctxt().kind(pointer) {
            TyKind::Ref { pointee, .. } => pointee,
            TyKind::Error => pointer,
            _ => {
                self.error(
                    codes::E3012,
                    span,
                    format!(
                        "cannot dereference a value of type `{}`",
                        self.ty_text(pointer)
                    ),
                )
                .help("only reference values (`*T` or `*mut T`) can be dereferenced")
                .emit();
                self.error_ty()
            }
        }
    }
}
