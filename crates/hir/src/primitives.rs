//! Semantic kinds of the primitive type names.
//!
//! Name resolution, type checking, and later phases share this single mapping
//! from syntax to [`TyKind`], so a primitive can never mean different things in
//! different phases. The C scalar names resolve against the target's C ABI;
//! every other name is target-independent.

use seal_ast::PrimitiveType;
use seal_ty::{CAbiProperties, FloatTy, IntTy, TyKind, UintTy};

/// Returns the semantic kind of `primitive` for `c_abi`.
pub fn primitive_kind(primitive: PrimitiveType, c_abi: CAbiProperties) -> TyKind {
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
        PrimitiveType::CChar => {
            if c_abi.char_signed() {
                TyKind::Int(IntTy::I8)
            } else {
                TyKind::Uint(UintTy::U8)
            }
        }
        PrimitiveType::CSchar => TyKind::Int(IntTy::I8),
        PrimitiveType::CUchar => TyKind::Uint(UintTy::U8),
        PrimitiveType::CShort => TyKind::Int(IntTy::I16),
        PrimitiveType::CUshort => TyKind::Uint(UintTy::U16),
        PrimitiveType::CInt => TyKind::Int(IntTy::I32),
        PrimitiveType::CUint => TyKind::Uint(UintTy::U32),
        PrimitiveType::CLong => c_long(c_abi, true),
        PrimitiveType::CUlong => c_long(c_abi, false),
        PrimitiveType::CLongLong => TyKind::Int(IntTy::I64),
        PrimitiveType::CUlongLong => TyKind::Uint(UintTy::U64),
        PrimitiveType::CFloat => TyKind::Float(FloatTy::F32),
        PrimitiveType::CDouble => TyKind::Float(FloatTy::F64),
        PrimitiveType::CBool => TyKind::Uint(UintTy::U8),
        PrimitiveType::CWchar => c_wchar(c_abi),
        PrimitiveType::SizeT => TyKind::Uint(UintTy::Usize),
        PrimitiveType::SsizeT => TyKind::Int(IntTy::Isize),
        PrimitiveType::PtrdiffT => TyKind::Int(IntTy::Isize),
        PrimitiveType::IntptrT => TyKind::Int(IntTy::Isize),
        PrimitiveType::UintptrT => TyKind::Uint(UintTy::Usize),
    }
}

/// Resolves `long` and `unsigned long`, whose width follows the target.
fn c_long(c_abi: CAbiProperties, signed: bool) -> TyKind {
    match (c_abi.long_width(), signed) {
        (32, true) => TyKind::Int(IntTy::I32),
        (32, false) => TyKind::Uint(UintTy::U32),
        (64, true) => TyKind::Int(IntTy::I64),
        (64, false) => TyKind::Uint(UintTy::U64),
        (other, _) => unreachable!("a C long is 32 or 64 bits wide, not {other}"),
    }
}

/// Resolves `wchar_t`, whose width and signedness follow the target.
fn c_wchar(c_abi: CAbiProperties) -> TyKind {
    match (c_abi.wchar_width(), c_abi.wchar_signed()) {
        (16, true) => TyKind::Int(IntTy::I16),
        (16, false) => TyKind::Uint(UintTy::U16),
        (32, true) => TyKind::Int(IntTy::I32),
        (32, false) => TyKind::Uint(UintTy::U32),
        (other, _) => unreachable!("a C wchar_t is 16 or 32 bits wide, not {other}"),
    }
}

#[cfg(test)]
mod tests {
    use seal_ast::PrimitiveType;
    use seal_ty::{CAbiProperties, IntTy, TyKind, UintTy};

    use super::primitive_kind;

    fn lp64() -> CAbiProperties {
        CAbiProperties::for_target(
            &seal_target::TargetTriple::parse("x86_64-unknown-linux-gnu")
                .expect("valid triple")
                .spec(),
        )
    }

    fn windows() -> CAbiProperties {
        CAbiProperties::for_target(
            &seal_target::TargetTriple::parse("x86_64-pc-windows-msvc")
                .expect("valid triple")
                .spec(),
        )
    }

    fn arm_linux() -> CAbiProperties {
        CAbiProperties::for_target(
            &seal_target::TargetTriple::parse("aarch64-unknown-linux-gnu")
                .expect("valid triple")
                .spec(),
        )
    }

    #[test]
    fn fixed_width_c_types_have_one_meaning() {
        let abi = lp64();
        assert_eq!(
            primitive_kind(PrimitiveType::CInt, abi),
            TyKind::Int(IntTy::I32)
        );
        assert_eq!(
            primitive_kind(PrimitiveType::CUint, abi),
            TyKind::Uint(UintTy::U32)
        );
        assert_eq!(
            primitive_kind(PrimitiveType::CShort, abi),
            TyKind::Int(IntTy::I16)
        );
        assert_eq!(
            primitive_kind(PrimitiveType::CLongLong, abi),
            TyKind::Int(IntTy::I64)
        );
        assert_eq!(
            primitive_kind(PrimitiveType::CUlongLong, abi),
            TyKind::Uint(UintTy::U64)
        );
        assert_eq!(
            primitive_kind(PrimitiveType::CBool, abi),
            TyKind::Uint(UintTy::U8)
        );
        assert_eq!(
            primitive_kind(PrimitiveType::SizeT, abi),
            TyKind::Uint(UintTy::Usize)
        );
        assert_eq!(
            primitive_kind(PrimitiveType::SsizeT, abi),
            TyKind::Int(IntTy::Isize)
        );
        assert_eq!(
            primitive_kind(PrimitiveType::PtrdiffT, abi),
            TyKind::Int(IntTy::Isize)
        );
        assert_eq!(
            primitive_kind(PrimitiveType::IntptrT, abi),
            TyKind::Int(IntTy::Isize)
        );
        assert_eq!(
            primitive_kind(PrimitiveType::UintptrT, abi),
            TyKind::Uint(UintTy::Usize)
        );
    }

    #[test]
    fn long_follows_the_target_data_model() {
        assert_eq!(
            primitive_kind(PrimitiveType::CLong, lp64()),
            TyKind::Int(IntTy::I64)
        );
        assert_eq!(
            primitive_kind(PrimitiveType::CUlong, lp64()),
            TyKind::Uint(UintTy::U64)
        );
        assert_eq!(
            primitive_kind(PrimitiveType::CLong, windows()),
            TyKind::Int(IntTy::I32)
        );
        assert_eq!(
            primitive_kind(PrimitiveType::CUlong, windows()),
            TyKind::Uint(UintTy::U32)
        );
    }

    #[test]
    fn plain_char_follows_the_target() {
        assert_eq!(
            primitive_kind(PrimitiveType::CChar, lp64()),
            TyKind::Int(IntTy::I8)
        );
        assert_eq!(
            primitive_kind(PrimitiveType::CChar, arm_linux()),
            TyKind::Uint(UintTy::U8)
        );
        assert_eq!(
            primitive_kind(PrimitiveType::CSchar, arm_linux()),
            TyKind::Int(IntTy::I8)
        );
        assert_eq!(
            primitive_kind(PrimitiveType::CUchar, lp64()),
            TyKind::Uint(UintTy::U8)
        );
    }

    #[test]
    fn wchar_follows_the_target() {
        assert_eq!(
            primitive_kind(PrimitiveType::CWchar, lp64()),
            TyKind::Int(IntTy::I32)
        );
        assert_eq!(
            primitive_kind(PrimitiveType::CWchar, windows()),
            TyKind::Uint(UintTy::U16)
        );
    }

    #[test]
    fn float_aliases_are_the_seal_floats() {
        assert_eq!(
            primitive_kind(PrimitiveType::CFloat, lp64()),
            TyKind::Float(seal_ty::FloatTy::F32)
        );
        assert_eq!(
            primitive_kind(PrimitiveType::CDouble, lp64()),
            TyKind::Float(seal_ty::FloatTy::F64)
        );
    }
}
