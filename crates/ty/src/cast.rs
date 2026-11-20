//! Classification of explicit primitive numeric casts.
//!
//! The `as` operator only converts between primitive numeric types. This
//! module owns the single table that decides which conversion a pair of types
//! performs, so type checking, MIR, and code generation never duplicate it.

use crate::kind::{IntTy, RefKind, UintTy};
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

/// The conversion performed by an explicit `as` cast.
///
/// Numeric conversions keep their own classification in [`NumericCastKind`].
/// Pointer conversions name what happens to the pointer representation so MIR
/// and code generation never re-derive the category from the endpoint types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CastKind {
    /// An integer or floating-point conversion.
    Numeric(NumericCastKind),
    /// A reference becomes a raw pointer with the same pointee.
    RefToRaw {
        /// Whether the result is `*raw mut T`.
        mutable: bool,
    },
    /// A raw pointer becomes another raw pointer.
    RawToRaw,
    /// A raw pointer becomes `usize` or `isize`.
    RawToInt,
    /// `usize` or `isize` becomes a thin raw pointer.
    IntToRaw,
    /// Reserved for a future explicit pointer-to-reference conversion.
    ///
    /// The surface rejects it: a reference is created with `&*pointer` or
    /// `&mut *pointer`, which registers a loan instead of bypassing the borrow
    /// checker through a cast.
    RawToRef,
}

impl TyCtxt {
    /// Classifies an explicit cast, or returns `None` when `as` cannot convert
    /// `from` to `to`.
    ///
    /// The table is structural: it decides pointer categories and metadata
    /// compatibility, and leaves trait-implementation proofs to type checking.
    pub fn cast_kind(&self, from: TyId, to: TyId) -> Option<CastKind> {
        if let Some(numeric) = self.numeric_cast_kind(from, to) {
            return Some(CastKind::Numeric(numeric));
        }
        match (self.kind(from), self.kind(to)) {
            (
                TyKind::Ref {
                    kind: source_kind,
                    pointee: source_pointee,
                },
                TyKind::RawPtr {
                    kind: raw_kind,
                    pointee: target_pointee,
                },
            ) => {
                if source_kind == RefKind::Shared && raw_kind.is_mut() {
                    return None;
                }
                let same_pointee = source_pointee == target_pointee;
                let unsizes_to_object = matches!(self.kind(target_pointee), TyKind::Dyn { .. })
                    && !matches!(self.kind(source_pointee), TyKind::Dyn { .. });
                if same_pointee || unsizes_to_object {
                    return Some(CastKind::RefToRaw {
                        mutable: raw_kind.is_mut(),
                    });
                }
                None
            }
            (
                TyKind::RawPtr {
                    pointee: source_pointee,
                    ..
                },
                TyKind::RawPtr {
                    pointee: target_pointee,
                    ..
                },
            ) => {
                match (
                    self.is_fat_pointee(source_pointee),
                    self.is_fat_pointee(target_pointee),
                ) {
                    (false, false) => Some(CastKind::RawToRaw),
                    (true, true) if self.same_pointer_metadata(source_pointee, target_pointee) => {
                        Some(CastKind::RawToRaw)
                    }
                    _ => None,
                }
            }
            (TyKind::RawPtr { .. }, TyKind::Uint(UintTy::Usize) | TyKind::Int(IntTy::Isize)) => {
                Some(CastKind::RawToInt)
            }
            (
                TyKind::Uint(UintTy::Usize) | TyKind::Int(IntTy::Isize),
                TyKind::RawPtr { pointee, .. },
            ) if !self.is_fat_pointee(pointee) => Some(CastKind::IntToRaw),
            _ => None,
        }
    }

    /// Returns `true` when a pointer's pointee carries metadata, so the pointer
    /// occupies two words.
    pub(crate) fn is_fat_pointee(&self, pointee: TyId) -> bool {
        matches!(
            self.kind(pointee),
            TyKind::Str | TyKind::Slice { .. } | TyKind::Dyn { .. }
        )
    }

    /// Returns `true` when two fat pointees carry compatible metadata.
    ///
    /// Any two slices share the length representation, and a trait object can
    /// only change to the same principal trait with the same arguments, because
    /// the vtable is selected by that pair.
    fn same_pointer_metadata(&self, source: TyId, target: TyId) -> bool {
        match (self.kind(source), self.kind(target)) {
            (TyKind::Str, TyKind::Str) => true,
            (TyKind::Slice { .. }, TyKind::Slice { .. }) => true,
            (
                TyKind::Dyn {
                    trait_def: source_trait,
                    args: source_args,
                },
                TyKind::Dyn {
                    trait_def: target_trait,
                    args: target_args,
                },
            ) => source_trait == target_trait && source_args == target_args,
            _ => false,
        }
    }

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
