//! Reference types: borrowing places and reading through references.

use seal_error_codes as codes;
use seal_hir::{ExprKind, HirExprId};
use seal_span::Span;
use seal_ty::{RefKind, TyId, TyKind};

use super::Checker;
use super::flow::Flow;

impl Checker<'_> {
    /// Types `&place` as `*T`, where `T` is the type of the borrowed place.
    ///
    /// A place is a binding, a parameter, a field, or a dereference of a
    /// reference — including a reference value such as a call result, because
    /// `*reference` names the pointee wherever the reference came from. A
    /// computed value has no storage, and Seal does not create hidden
    /// temporaries for borrows. Reading the borrowed place also requires it to
    /// be initialized, which `infer_expr` checks.
    pub(super) fn check_borrow(
        &mut self,
        operand: HirExprId,
        kind: RefKind,
        flow: &mut Flow,
    ) -> TyId {
        let is_place = self.hir.place(operand).is_some()
            || matches!(self.hir.expr(operand).kind, ExprKind::Deref { .. });
        let ty = self.infer_expr(operand, None, flow);

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
        self.ref_ty(kind, ty)
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
                .help("only `*T` values can be dereferenced")
                .emit();
                self.error_ty()
            }
        }
    }
}
