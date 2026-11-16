//! Type lowering.

use seal_ast::{PrimitiveType, TypeId, TypeKind};
use seal_ty::{FloatTy, IntTy, TyId, UintTy};

use super::Lowering;

impl Lowering<'_> {
    /// Lowers a type annotation into the shared type context.
    ///
    /// Every occurrence of the same type returns the same [`TyId`], so the
    /// checker can compare types by identity.
    pub(super) fn lower_type(&mut self, ty: TypeId) -> TyId {
        match self.ast.ty(ty).kind {
            TypeKind::Primitive(primitive) => self.lower_primitive(primitive),
            TypeKind::Pointer(inner) => {
                let inner = self.lower_type(inner);
                self.hir.ty_ctxt_mut().ref_ty(inner)
            }
            TypeKind::Error => self.hir.ty_ctxt_mut().error_ty(),
        }
    }

    fn lower_primitive(&mut self, primitive: PrimitiveType) -> TyId {
        let ctxt = self.hir.ty_ctxt_mut();
        match primitive {
            PrimitiveType::I8 => ctxt.int_ty(IntTy::I8),
            PrimitiveType::I16 => ctxt.int_ty(IntTy::I16),
            PrimitiveType::I32 => ctxt.int_ty(IntTy::I32),
            PrimitiveType::I64 => ctxt.int_ty(IntTy::I64),
            PrimitiveType::I128 => ctxt.int_ty(IntTy::I128),
            PrimitiveType::Isize => ctxt.int_ty(IntTy::Isize),
            PrimitiveType::U8 => ctxt.uint_ty(UintTy::U8),
            PrimitiveType::U16 => ctxt.uint_ty(UintTy::U16),
            PrimitiveType::U32 => ctxt.uint_ty(UintTy::U32),
            PrimitiveType::U64 => ctxt.uint_ty(UintTy::U64),
            PrimitiveType::U128 => ctxt.uint_ty(UintTy::U128),
            PrimitiveType::Usize => ctxt.uint_ty(UintTy::Usize),
            PrimitiveType::F32 => ctxt.float_ty(FloatTy::F32),
            PrimitiveType::F64 => ctxt.float_ty(FloatTy::F64),
            PrimitiveType::Bool => ctxt.bool_ty(),
            PrimitiveType::Char => ctxt.char_ty(),
            PrimitiveType::Str => ctxt.str_ty(),
        }
    }
}
