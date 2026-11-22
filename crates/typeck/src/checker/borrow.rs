//! Reference types: borrowing places and reading through references.

use seal_error_codes as codes;
use seal_hir::{ExprKind, HirExprId};
use seal_span::Span;
use seal_ty::{RawPtrKind, RefKind, TyId, TyKind};

use super::flow::Flow;
use super::{Checker, PlaceMode};
use crate::results::Coercion;

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
        expr: HirExprId,
        operand: HirExprId,
        kind: RefKind,
        expected: Option<TyId>,
        flow: &mut Flow,
    ) -> TyId {
        let is_place = self.hir.place(operand).is_some()
            || matches!(self.hir.expr(operand).kind, ExprKind::Deref { .. });
        let mode = if kind.is_exclusive() {
            PlaceMode::BorrowExclusive
        } else {
            PlaceMode::Borrow
        };
        // A borrow is the one context where a direct raw dereference may be
        // looking for a checked view. Let the dereference type its pointee, so
        // the borrow can report the precise `owned_view` diagnostic instead of
        // the generic "slice value" error.
        let previous_place_base = self.checking_place_base;
        if matches!(self.hir.expr(operand).kind, ExprKind::Deref { .. }) {
            self.checking_place_base = true;
        }
        let ty = self.infer_expr_mode(operand, None, flow, mode);
        self.checking_place_base = previous_place_base;

        if !is_place {
            if !self.is_error(ty) {
                let span = self.hir.expr(operand).span;
                let function = matches!(
                    self.hir.expr(operand).kind,
                    ExprKind::Path {
                        res: seal_hir::Res::Function(_) | seal_hir::Res::TraitFunction { .. },
                        ..
                    }
                );
                if function {
                    self.error(
                        codes::E3013,
                        span,
                        "a function item is already a pointer, so it cannot be borrowed",
                    )
                    .help("write the function name without `&`")
                    .emit();
                } else {
                    self.error(codes::E3013, span, "cannot borrow this expression")
                        .help("borrow a place, such as a binding or a parameter")
                        .emit();
                }
            }
            return self.error_ty();
        }

        if let Some(pointee) = self.bare_fat_raw_borrow_pointee(operand)
            && matches!(
                self.hir.ty_ctxt().kind(pointee),
                TyKind::Slice { .. } | TyKind::Str | TyKind::Dyn { .. }
            )
        {
            self.error(
                codes::E3106,
                self.hir.expr(operand).span,
                "cannot borrow a fat raw pointer as a safe view",
            )
            .help(
                "use `owned_view(owner, pointer)` so the allocation owner stays borrowed, or a \
                 container method such as `Box::as_ref`",
            )
            .emit();
            return self.error_ty();
        }
        if self.is_error(ty) {
            return ty;
        }
        if matches!(self.hir.ty_ctxt().kind(ty), TyKind::Dyn { .. }) {
            return self.check_dynamic_borrow(expr, operand, kind, ty);
        }
        if !self.is_sized(ty) && !matches!(self.hir.ty_ctxt().kind(ty), TyKind::Str) {
            self.error(
                codes::E3024,
                self.hir.expr(operand).span,
                format!("cannot borrow the unsized type `{}`", self.ty_text(ty)),
            )
            .help("borrow a slice range instead, as in `&values[..]`")
            .emit();
            return self.error_ty();
        }
        if kind.is_exclusive() && !self.borrow_is_writable(operand) {
            self.report_mutable_borrow(operand);
            return self.error_ty();
        }

        // An array reference coerces to a slice reference when the context
        // requires one. The coercion is driven by the expected type, so
        // `let s: *[i32] = &values;` sees `*[i32]`, while `let r = &values;`
        // keeps the full array type.
        let reference = self.ref_ty(kind, ty);
        if let Some(expected) = expected
            && let TyKind::Ref {
                kind: expected_kind,
                pointee,
            } = self.hir.ty_ctxt().kind(expected)
            && expected_kind == kind
            && let Some(expected_element) = self.slice_parts(pointee)
            && let Some((element, _)) = self.array_parts(ty)
            && expected_element == element
        {
            return expected;
        }
        reference
    }

    /// Reborrows a dyn place reached through a trait-object pointer.
    ///
    /// The new reference copies the data and vtable pair. It never creates a new
    /// vtable or inspects the concrete type, so the coercion records the source
    /// pointer type for later lowering.
    fn check_dynamic_borrow(
        &mut self,
        expr: HirExprId,
        operand: HirExprId,
        kind: RefKind,
        object: TyId,
    ) -> TyId {
        let Some((pointer_ty, pointer_kind)) = self.dyn_pointer_source(operand) else {
            self.error(
                codes::E3024,
                self.hir.expr(operand).span,
                "cannot borrow a bare trait-object place",
            )
            .help("borrow through a trait-object pointer, as in `&*pointer`")
            .emit();
            return self.error_ty();
        };
        if kind.is_exclusive() && !pointer_kind {
            self.report_mutable_borrow(operand);
            return self.error_ty();
        }
        let target = self.ref_ty(kind, object);
        self.results.record_coercion(
            expr,
            Coercion::TraitObjectReborrow {
                source: pointer_ty,
                target,
            },
        );
        target
    }

    /// Returns the pointer type and kind behind `*pointer`, when the pointer
    /// reaches a trait object.
    fn dyn_pointer_source(&self, operand: HirExprId) -> Option<(TyId, bool)> {
        let ExprKind::Deref { operand: pointer } = self.hir.expr(operand).kind else {
            return None;
        };
        let pointer_ty = self.results.expr_ty(pointer)?;
        let tcx = self.hir.ty_ctxt();
        if let Some((kind, pointee)) = tcx.ref_parts(pointer_ty) {
            return matches!(tcx.kind(pointee), TyKind::Dyn { .. })
                .then_some((pointer_ty, kind.is_exclusive()));
        }
        let (kind, pointee) = tcx.raw_parts(pointer_ty)?;
        matches!(tcx.kind(pointee), TyKind::Dyn { .. }).then_some((pointer_ty, kind.is_mut()))
    }

    /// Returns the pointee of a raw pointer dereferenced directly by a borrow.
    ///
    /// `&*pointer` and `&mut *pointer` have no owner place, so they can only
    /// produce a thin reference. A fat raw pointer needs its metadata copied
    /// and its allocation owner loaned, which is the `owned_view` contract.
    fn bare_fat_raw_borrow_pointee(&self, operand: HirExprId) -> Option<TyId> {
        let ExprKind::Deref { operand: pointer } = self.hir.expr(operand).kind else {
            return None;
        };
        let pointer_ty = self.results.expr_ty(pointer)?;
        let (_, pointee) = self.hir.ty_ctxt().raw_parts(pointer_ty)?;
        Some(pointee)
    }

    /// Returns `true` when `operand` names a place an exclusive borrow may
    /// write.
    pub(super) fn borrow_is_writable(&self, operand: HirExprId) -> bool {
        if let Some(kind) = self.raw_deref_kind(operand) {
            return kind.is_mut();
        }
        match self.deref_kind(operand) {
            Some(kind) => kind.is_exclusive(),
            None => self
                .results
                .place(operand)
                .is_some_and(|place| self.root_is_writable(place.root)),
        }
    }

    /// Returns the kind of the last raw-pointer dereference `expr` reads
    /// through, if the last dereference is raw.
    pub(super) fn raw_deref_kind(&self, expr: HirExprId) -> Option<RawPtrKind> {
        match self.results.place(expr) {
            Some(place) if place.contains_raw_deref() => place.raw_deref,
            Some(_) => None,
            None => match self.hir.expr(expr).kind {
                ExprKind::Deref { operand } => self.raw_reference_kind(operand),
                _ => None,
            },
        }
    }

    /// Returns the kind of the raw pointer type of `expr`, if it is one.
    fn raw_reference_kind(&self, expr: HirExprId) -> Option<RawPtrKind> {
        let ty = self.results.expr_ty(expr)?;
        self.hir.ty_ctxt().raw_parts(ty).map(|(kind, _)| kind)
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

    pub(super) fn report_mutable_borrow(&self, operand: HirExprId) {
        if let Some(kind) = self.raw_deref_kind(operand) {
            if !kind.is_mut() {
                self.error(
                    codes::E3075,
                    self.hir.expr(operand).span,
                    "cannot borrow through a `*raw T` pointer as mutable",
                )
                .help("`*raw T` is read-only; declare the pointer as `*raw mut T`")
                .emit();
            }
            return;
        }
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
        let place = self.results.place(operand);
        let text = place.map_or_else(|| "this place".to_owned(), |place| self.place_text(place));
        if let Some(place) = place
            && !place.root.is_local()
        {
            self.error(
                codes::E3017,
                self.hir.expr(operand).span,
                format!("cannot borrow the immutable item `{text}` as mutable"),
            )
            .help("an immutable `const` or `static` is read-only")
            .emit();
            return;
        }
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
            TyKind::Ref { pointee, .. }
                if matches!(self.hir.ty_ctxt().kind(pointee), TyKind::Slice { .. })
                    && !self.checking_place_base =>
            {
                self.error(
                    codes::E3024,
                    span,
                    "cannot read through a slice reference as a value",
                )
                .help("index the slice, as in `values[0]`, or borrow a range such as `&values[..]`")
                .emit();
                self.error_ty()
            }
            TyKind::Ref { pointee, .. } => pointee,
            TyKind::RawPtr { pointee, .. } => {
                if self.unsafe_depth == 0 {
                    self.error(
                        codes::E3070,
                        span,
                        "dereferencing a raw pointer requires an `unsafe` block",
                    )
                    .help("wrap the dereference in `unsafe { ... }`")
                    .emit();
                }
                if matches!(self.hir.ty_ctxt().kind(pointee), TyKind::Opaque { .. }) {
                    self.error(
                        codes::E3087,
                        span,
                        format!(
                            "cannot dereference the opaque type `{}`; Seal does not know its layout",
                            self.ty_text(pointee)
                        ),
                    )
                    .help("keep the pointer and pass it back to C")
                    .emit();
                    return self.error_ty();
                }
                if matches!(self.hir.ty_ctxt().kind(pointee), TyKind::Slice { .. })
                    && !self.checking_place_base
                {
                    self.error(
                        codes::E3024,
                        span,
                        "cannot read through a slice pointer as a value",
                    )
                    .help("index the slice, as in `(*pointer)[0]`, or use `data()` and `len()`")
                    .emit();
                    self.error_ty()
                } else {
                    pointee
                }
            }
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
                .help("only reference or raw pointer values can be dereferenced")
                .emit();
                self.error_ty()
            }
        }
    }
}
