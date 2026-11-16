//! Type lowering.

use seal_ast::{TypeId, TypeKind};
use seal_ty::{TyCtxt, TyId};

use super::Lowering;
use crate::primitives::primitive_kind;

impl Lowering<'_> {
    /// Lowers a type annotation into the shared type context.
    ///
    /// Every occurrence of the same type returns the same [`TyId`], so the
    /// checker can compare types by identity.
    pub(super) fn lower_type(&mut self, ty: TypeId) -> TyId {
        match self.ast.ty(ty).kind {
            TypeKind::Primitive(primitive) => {
                self.hir.ty_ctxt_mut().intern(primitive_kind(primitive))
            }
            TypeKind::Pointer(inner) => {
                let inner = self.lower_type(inner);
                self.hir.ty_ctxt_mut().ref_ty(inner)
            }
            TypeKind::Error => self.hir.ty_ctxt_mut().error_ty(),
        }
    }
}

/// Interns every type the checker can create without mutating the context.
///
/// The checker does not own the type context, so lowering guarantees these
/// canonical types exist: all primitive scalars, `()`, `!`, the error type, and
/// the string-literal type `*str`. A later phase that introduces aggregate types
/// will extend this set or take the context mutably.
pub(crate) fn intern_primitives(ctxt: &mut TyCtxt) {
    for primitive in seal_ast::PrimitiveType::ALL {
        ctxt.intern(primitive_kind(primitive));
    }
    ctxt.unit_ty();
    ctxt.never_ty();
    ctxt.error_ty();
    let str_ty = ctxt.str_ty();
    ctxt.ref_ty(str_ty);
}
