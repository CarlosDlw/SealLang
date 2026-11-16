//! Semantic kinds of the primitive type names.
//!
//! Name resolution, type checking, and later phases share this single mapping
//! from syntax to [`TyKind`], so a primitive can never mean different things in
//! different phases.

use seal_ast::PrimitiveType;
use seal_ty::{FloatTy, IntTy, TyKind, UintTy};

/// Returns the semantic kind of `primitive`.
pub fn primitive_kind(primitive: PrimitiveType) -> TyKind {
    match primitive {
        PrimitiveType::I8 => TyKind::Int(IntTy::I8),
        PrimitiveType::I16 => TyKind::Int(IntTy::I16),
        PrimitiveType::I32 => TyKind::Int(IntTy::I32),
        PrimitiveType::I64 => TyKind::Int(IntTy::I64),
        PrimitiveType::I128 => TyKind::Int(IntTy::I128),
        PrimitiveType::Isize => TyKind::Int(IntTy::Isize),
        PrimitiveType::U8 => TyKind::Uint(UintTy::U8),
        PrimitiveType::U16 => TyKind::Uint(UintTy::U16),
        PrimitiveType::U32 => TyKind::Uint(UintTy::U32),
        PrimitiveType::U64 => TyKind::Uint(UintTy::U64),
        PrimitiveType::U128 => TyKind::Uint(UintTy::U128),
        PrimitiveType::Usize => TyKind::Uint(UintTy::Usize),
        PrimitiveType::F32 => TyKind::Float(FloatTy::F32),
        PrimitiveType::F64 => TyKind::Float(FloatTy::F64),
        PrimitiveType::Bool => TyKind::Bool,
        PrimitiveType::Char => TyKind::Char,
        PrimitiveType::Str => TyKind::Str,
    }
}
