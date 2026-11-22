//! The unsafe `owned_view(owner, source)` builtin.
//!
//! The builtin ties a raw pointer's representation to an owner place: the
//! returned reference borrows `owner`, not the pointer local. Type checking
//! establishes the structural contract; borrow checking turns the owner place
//! into a loan and keeps the owner live while the view is used.

use seal_error_codes as codes;
use seal_hir::HirExprId;
use seal_span::Span;
use seal_ty::{RawPtrKind, RefKind, TyId, TyKind};

use super::flow::Flow;
use super::{Checker, PlaceMode};
use crate::results::PlaceRoot;

impl Checker<'_> {
    /// Types `unsafe { owned_view(owner, source) }`.
    ///
    /// `owner` must be an initialized place that can carry a loan. A checked
    /// dereference of a temporary reference is also accepted: MIR materializes
    /// that reference in a temporary and promotes it to the enclosing block, so
    /// the owner storage remains live for the view. `source` must be a raw
    /// pointer whose pointee has a concrete representation. Its mutability
    /// selects the reference kind, so a shared raw pointer can never become a
    /// mutable view implicitly.
    pub(super) fn check_owned_view(
        &mut self,
        owner: HirExprId,
        source: HirExprId,
        span: Span,
        flow: &mut Flow,
    ) -> TyId {
        let owner_ty = self.infer_expr_mode(owner, None, flow, PlaceMode::Borrow);
        let source_ty = self.infer_expr(source, None, flow);
        if self.is_error(owner_ty) || self.is_error(source_ty) {
            return self.error_ty();
        }

        if self.unsafe_depth == 0 {
            self.error(
                codes::E3070,
                span,
                "creating an owned view requires an `unsafe` block",
            )
            .help(
                "wrap the call in `unsafe { ... }` and establish the allocation, alignment, \
                 initialization, and aliasing contract",
            )
            .emit();
            return self.error_ty();
        }

        let owner_span = self.hir.expr(owner).span;
        let place = self.results.place(owner).cloned();
        // A dereference of a reference value has no resolved place, because
        // the reference itself is a temporary. MIR materializes and promotes
        // that temporary, so the dereference is still checked storage. Raw
        // dereferences have no reference kind and stay rejected below.
        let temporary_deref = place.is_none() && self.deref_kind(owner).is_some();
        if place.is_none() && !temporary_deref {
            if self.raw_deref_kind(owner).is_some() {
                self.error(
                    codes::E3106,
                    owner_span,
                    "an `owned_view` owner cannot be reached through a raw pointer",
                )
                .help(
                    "the owner must be checked storage, such as a local, a static, or `*reference`",
                )
                .emit();
            } else {
                self.error(
                    codes::E3106,
                    owner_span,
                    "the first argument of `owned_view` must be an owner place",
                )
                .help("borrow the value that owns the allocation, not a temporary or a call result")
                .emit();
            }
            return self.error_ty();
        }
        if let Some(place) = &place {
            if matches!(place.root, PlaceRoot::Const(_)) {
                self.error(
                    codes::E3106,
                    owner_span,
                    "a constant cannot own the storage of an `owned_view`",
                )
                .help("keep the allocation in a local, a static, or a borrowed container")
                .emit();
                return self.error_ty();
            }
            if place.contains_raw_deref() {
                self.error(
                    codes::E3106,
                    owner_span,
                    "an `owned_view` owner cannot be reached through a raw pointer",
                )
                .help(
                    "the owner must be checked storage, such as a local, a static, or `*reference`",
                )
                .emit();
                return self.error_ty();
            }
        }

        let source_span = self.hir.expr(source).span;
        let TyKind::RawPtr { kind, pointee } = self.hir.ty_ctxt().kind(source_ty) else {
            self.error(
                codes::E3106,
                source_span,
                format!(
                    "the second argument of `owned_view` must be a raw pointer, found `{}`",
                    self.ty_text(source_ty)
                ),
            )
            .help("cast the address to `*raw T` or `*raw mut T` before creating the view")
            .emit();
            return self.error_ty();
        };
        if !self.owned_view_pointee_supported(pointee) {
            self.error(
                codes::E3106,
                source_span,
                format!(
                    "`owned_view` cannot view the unsupported pointee type `{}`",
                    self.ty_text(pointee)
                ),
            )
            .help("use a sized pointee, `[T]`, or `str`")
            .emit();
            return self.error_ty();
        }

        let kind = match kind {
            RawPtrKind::Const => RefKind::Shared,
            RawPtrKind::Mut => RefKind::Exclusive,
        };
        if kind.is_exclusive() && !self.borrow_is_writable(owner) {
            self.report_mutable_borrow(owner);
            return self.error_ty();
        }

        self.ref_ty(kind, pointee)
    }

    /// Returns whether a raw pointee has a reference representation.
    ///
    /// `str` and slices are the two fat pointees this primitive supports.
    /// Trait objects need a vtable and are reserved for a later extension;
    /// opaque and function-signature types have no layout at all.
    fn owned_view_pointee_supported(&self, pointee: TyId) -> bool {
        match self.hir.ty_ctxt().kind(pointee) {
            TyKind::Str | TyKind::Slice { .. } => true,
            TyKind::Error
            | TyKind::Infer(_)
            | TyKind::Opaque { .. }
            | TyKind::Dyn { .. }
            | TyKind::FnSig(_)
            | TyKind::Never => false,
            _ => self.is_sized(pointee),
        }
    }
}
