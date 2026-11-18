//! Classification of explicit primitive numeric casts.
//!
//! The `as` operator only converts between primitive numeric types. This
//! module owns the single table that decides which conversion a pair of types
//! performs, so type checking, MIR, and code generation never duplicate it.

use crate::{TyCtxt, TyId, TyKind};

/// The kind of primitive numeric conversion performed by `as`.
///
/// This is deliberately narrower than a general cast kind: bool, char, enum,
/// pointer, and aggregate conversions get their own semantic categories when
/// they are designed, instead of widening the meaning of these variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NumericCastKind {
    /// An integer converted to another integer, by truncation or extension.
    IntToInt,
    /// An integer converted to a floating-point value, preserving its sign.
    IntToFloat,
    /// A floating-point value truncated toward zero and saturated to an integer.
    FloatToInt,
    /// A floating-point value rounded to another floating-point width.
    FloatToFloat,
}

impl TyCtxt {
    /// Classifies a numeric cast, or returns `None` when either type is not a
    /// concrete integer or float.
    ///
    /// `Error`, `Infer`, `Param`, and every non-numeric kind return `None`;
    /// callers decide whether that is a diagnostic or an internal error.
    pub fn numeric_cast_kind(&self, from: TyId, to: TyId) -> Option<NumericCastKind> {
        let from = self.kind(from);
        let to = self.kind(to);
        match (
            from.is_integer(),
            from.is_float(),
            to.is_integer(),
            to.is_float(),
        ) {
            (true, false, true, false) => Some(NumericCastKind::IntToInt),
            (true, false, false, true) => Some(NumericCastKind::IntToFloat),
            (false, true, true, false) => Some(NumericCastKind::FloatToInt),
            (false, true, false, true) => Some(NumericCastKind::FloatToFloat),
            _ => None,
        }
    }
}

impl TyKind {
    /// Returns `true` for every signed and unsigned integer type.
    pub(crate) fn is_integer(self) -> bool {
        matches!(self, TyKind::Int(_) | TyKind::Uint(_))
    }

    /// Returns `true` for `f32` and `f64`.
    pub(crate) fn is_float(self) -> bool {
        matches!(self, TyKind::Float(_))
    }
}

#[cfg(test)]
mod tests {
    use crate::kind::{FloatTy, IntTy, RefKind, TyKind, UintTy};
    use crate::{InferCtxt, NumericCastKind, TyCtxt};

    #[test]
    fn every_numeric_family_is_classified() {
        let ctxt = TyCtxt::new();
        let i32 = ctxt.int_ty(IntTy::I32);
        let u8 = ctxt.uint_ty(UintTy::U8);
        let f32 = ctxt.float_ty(FloatTy::F32);
        let f64 = ctxt.float_ty(FloatTy::F64);

        assert_eq!(
            ctxt.numeric_cast_kind(i32, u8),
            Some(NumericCastKind::IntToInt)
        );
        assert_eq!(
            ctxt.numeric_cast_kind(u8, i32),
            Some(NumericCastKind::IntToInt)
        );
        assert_eq!(
            ctxt.numeric_cast_kind(i32, i32),
            Some(NumericCastKind::IntToInt)
        );
        assert_eq!(
            ctxt.numeric_cast_kind(i32, f64),
            Some(NumericCastKind::IntToFloat)
        );
        assert_eq!(
            ctxt.numeric_cast_kind(u8, f32),
            Some(NumericCastKind::IntToFloat)
        );
        assert_eq!(
            ctxt.numeric_cast_kind(f64, i32),
            Some(NumericCastKind::FloatToInt)
        );
        assert_eq!(
            ctxt.numeric_cast_kind(f32, u8),
            Some(NumericCastKind::FloatToInt)
        );
        assert_eq!(
            ctxt.numeric_cast_kind(f32, f64),
            Some(NumericCastKind::FloatToFloat)
        );
        assert_eq!(
            ctxt.numeric_cast_kind(f64, f32),
            Some(NumericCastKind::FloatToFloat)
        );
        assert_eq!(
            ctxt.numeric_cast_kind(f64, f64),
            Some(NumericCastKind::FloatToFloat)
        );
    }

    #[test]
    fn non_numeric_types_are_not_classified() {
        let ctxt = TyCtxt::new();
        let i32 = ctxt.int_ty(IntTy::I32);
        let non_numeric = [
            ctxt.bool_ty(),
            ctxt.char_ty(),
            ctxt.str_ty(),
            ctxt.unit_ty(),
            ctxt.never_ty(),
            ctxt.error_ty(),
            ctxt.ref_ty(i32),
            ctxt.mut_ref_ty(i32),
        ];
        for ty in non_numeric {
            assert_eq!(ctxt.numeric_cast_kind(ty, i32), None);
            assert_eq!(ctxt.numeric_cast_kind(i32, ty), None);
            assert_eq!(ctxt.numeric_cast_kind(ty, ty), None);
        }
        let pointer_target = ctxt.ref_ty(i32);
        assert_eq!(
            ctxt.kind(pointer_target),
            TyKind::Ref {
                kind: RefKind::Shared,
                pointee: i32
            }
        );
    }

    #[test]
    fn unresolved_types_are_not_classified() {
        let ctxt = TyCtxt::new();
        let i32 = ctxt.int_ty(IntTy::I32);
        let mut infer_ctxt = InferCtxt::new();
        let infer = infer_ctxt.fresh_ty_var(&ctxt);
        assert_eq!(ctxt.numeric_cast_kind(infer, i32), None);
        assert_eq!(ctxt.numeric_cast_kind(i32, infer), None);
        assert_eq!(ctxt.numeric_cast_kind(infer, infer), None);
    }
}
