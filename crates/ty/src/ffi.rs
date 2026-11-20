//! FFI-safety classification for types that cross a C boundary.
//!
//! A type is FFI-safe when its representation is the representation of a C
//! type on every supported target. The classification is deliberately
//! conservative: a type the compiler cannot describe to C is rejected instead
//! of being passed with an assumed layout.
//!
//! The predicate is shared by every boundary in the language: parameters,
//! returns, statics, aggregate fields, variadic arguments, and function
//! pointers. Keeping one predicate is what makes the answer independent of
//! where the type appears.

use std::fmt;

use crate::{CastKind, IntTy, NumericCastKind, RefKind, TyCtxt, TyId, TyKind, UintTy};

/// Where a type appears on a C boundary.
///
/// The position matters only for `()`: it is a valid return type (`void`) and
/// an invalid parameter or field. Every other rule is position-independent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FfiPosition {
    /// A function parameter.
    Parameter,
    /// A function return type.
    Return,
    /// The type of an `extern` static.
    Static,
    /// A field of an `extern` aggregate.
    Field,
    /// An extra argument after the fixed parameters of a variadic call.
    VariadicArgument,
    /// A parameter or return of a C function pointer type.
    FunctionPointer,
}

/// Why a type cannot cross a C boundary.
///
/// The reason is stable and testable; the human-readable text and the
/// suggestion belong to [`NotFfiSafe::reason`] and [`NotFfiSafe::suggestion`],
/// so a diagnostic never re-derives them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NotFfiSafe {
    /// A checked reference (`*T` or `*mut T`).
    Reference(RefKind),
    /// A pointer with slice, string, or trait-object metadata.
    FatPointer,
    /// `str`, `[T]`, or `dyn Trait` used by value.
    Unsized,
    /// A tuple, which has no C aggregate equivalent.
    Tuple,
    /// `char`, a Unicode scalar value rather than a C character.
    Char,
    /// A 128-bit integer, which has no standard C type.
    WideInteger,
    /// A Seal aggregate without a declared C layout.
    Aggregate,
    /// A type parameter, which has no representation before instantiation.
    Generic,
    /// An inference variable or an error-recovery type.
    Unresolved,
    /// `()` used anywhere other than as a return type.
    UnitParameter,
    /// The never type, which has no C return type.
    Never,
    /// An opaque type, which has no size or layout.
    Opaque,
    /// A zero-length array, which has no portable C representation.
    EmptyArray,
}

impl NotFfiSafe {
    /// Returns the sentence a diagnostic uses to explain the rejection.
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Reference(RefKind::Shared) => "a shared reference is not a C pointer",
            Self::Reference(RefKind::Exclusive) => "an exclusive reference is not a C pointer",
            Self::FatPointer => "a fat pointer carries metadata C does not have",
            Self::Unsized => "an unsized type cannot be passed by value",
            Self::Tuple => "a tuple has no C layout",
            Self::Char => "`char` is a Unicode scalar value, not a C character",
            Self::WideInteger => "C has no standard 128-bit integer",
            Self::Aggregate => "a Seal aggregate has no declared C layout",
            Self::Generic => "a type parameter has no C representation",
            Self::Unresolved => "an unresolved type cannot cross a C boundary",
            Self::UnitParameter => "`()` is only valid as a return type",
            Self::Never => "the never type has no C return type",
            Self::Opaque => "an opaque type has no size or C layout",
            Self::EmptyArray => "a zero-length array has no C representation",
        }
    }

    /// Returns the action that usually makes the type FFI-safe.
    pub const fn suggestion(self) -> &'static str {
        match self {
            Self::Reference(_) => "use `*raw T` or `*raw mut T`",
            Self::FatPointer => "pass the data pointer and the length separately",
            Self::Unsized => "pass a pointer and a length instead",
            Self::Tuple => "declare a C struct with `extern struct`",
            Self::Char => "use `u32` for a code point or `c_char` for a C character",
            Self::WideInteger => "use a 64-bit integer or split the value",
            Self::Aggregate => "declare the aggregate with `extern struct`",
            Self::Generic => "instantiate the declaration with a concrete type",
            Self::Unresolved => "write the concrete type",
            Self::UnitParameter => "return `()` only from a function that returns `void`",
            Self::Never => "return a type C can represent",
            Self::Opaque => "use `*raw T` to pass C-owned storage indirectly",
            Self::EmptyArray => "use a non-empty array",
        }
    }
}

impl TyCtxt {
    /// Returns the type an argument is promoted to when passed through `...`.
    ///
    /// C applies the default argument promotions: `bool` and integers narrower
    /// than `int` become `int`, and `f32` becomes `f64`. Wider integers, `f64`,
    /// and thin raw pointers keep their type. Every other type is not a valid
    /// variadic argument and returns `None`.
    pub fn variadic_promotion(&self, ty: TyId) -> Option<TyId> {
        match self.kind(ty) {
            TyKind::Bool => Some(self.int_ty(IntTy::I32)),
            TyKind::Int(int) => match int.bits() {
                Some(bits) if bits < 32 => Some(self.int_ty(IntTy::I32)),
                Some(bits) if bits <= 64 => Some(ty),
                // C has no standard variadic type for 128-bit integers.
                _ => None,
            },
            TyKind::Uint(uint) => match uint.bits() {
                // C promotes the narrow unsigned types to signed `int`.
                Some(bits) if bits < 32 => Some(self.int_ty(IntTy::I32)),
                Some(bits) if bits <= 64 => Some(ty),
                _ => None,
            },
            TyKind::Float(crate::FloatTy::F32) => Some(self.float_ty(crate::FloatTy::F64)),
            TyKind::Float(_) => Some(ty),
            TyKind::RawPtr { pointee, .. } if !self.is_fat_pointee(pointee) => Some(ty),
            _ => None,
        }
    }

    /// Returns the MIR conversion that performs one variadic promotion.
    ///
    /// C's default promotions are internal, not surface `as` casts, so `bool`
    /// promotes to `int` through the same zero-extension as an unsigned
    /// integer even though `bool as i32` is not part of the language. Every
    /// other promoted pair is classified by [`TyCtxt::cast_kind`].
    pub fn variadic_cast_kind(&self, from: TyId, to: TyId) -> Option<CastKind> {
        if self.variadic_promotion(from) != Some(to) {
            return None;
        }
        match self.kind(from) {
            TyKind::Bool => Some(CastKind::Numeric(NumericCastKind::IntToInt)),
            _ => self.cast_kind(from, to),
        }
    }
}

impl fmt::Display for NotFfiSafe {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason())
    }
}

/// Checks whether `ty` may cross a C boundary at `position`.
///
/// Thin raw pointers are always FFI-safe: their representation is one C
/// pointer word, whatever the pointee is. A pointer to `str`, a slice, or a
/// trait object is fat and is rejected, because C has no second metadata word
/// to receive.
pub fn safety(tcx: &TyCtxt, ty: TyId, position: FfiPosition) -> Result<(), NotFfiSafe> {
    match tcx.kind(ty) {
        TyKind::Bool | TyKind::Float(_) => Ok(()),
        TyKind::Int(IntTy::I128) | TyKind::Uint(UintTy::U128) => Err(NotFfiSafe::WideInteger),
        TyKind::Int(_) | TyKind::Uint(_) => Ok(()),
        TyKind::Char => Err(NotFfiSafe::Char),
        TyKind::Unit if position == FfiPosition::Return => Ok(()),
        TyKind::Unit => Err(NotFfiSafe::UnitParameter),
        TyKind::Never => Err(NotFfiSafe::Never),
        TyKind::Opaque { .. } => Err(NotFfiSafe::Opaque),
        TyKind::Str | TyKind::Slice { .. } | TyKind::Dyn { .. } => Err(NotFfiSafe::Unsized),
        TyKind::Ref { kind, .. } => Err(NotFfiSafe::Reference(kind)),
        TyKind::RawPtr { pointee, .. } => match tcx.kind(pointee) {
            TyKind::Str | TyKind::Slice { .. } | TyKind::Dyn { .. } => Err(NotFfiSafe::FatPointer),
            _ => Ok(()),
        },
        TyKind::Array { element, length } => {
            if length == 0 {
                return Err(NotFfiSafe::EmptyArray);
            }
            safety(tcx, element, position)
        }
        TyKind::Tuple(_) => Err(NotFfiSafe::Tuple),
        TyKind::Adt { adt, .. } => {
            if tcx.adt(adt).is_c_repr() {
                Ok(())
            } else {
                Err(NotFfiSafe::Aggregate)
            }
        }
        TyKind::Param(_) => Err(NotFfiSafe::Generic),
        TyKind::Infer(_) | TyKind::Error => Err(NotFfiSafe::Unresolved),
    }
}

#[cfg(test)]
mod tests {
    use super::{FfiPosition, NotFfiSafe, safety};
    use crate::{RawPtrKind, RefKind, TyCtxt, TyKind};

    fn scalar_kinds() -> Vec<TyKind> {
        vec![
            TyKind::Bool,
            TyKind::Int(crate::IntTy::I32),
            TyKind::Uint(crate::UintTy::U64),
            TyKind::Float(crate::FloatTy::F64),
        ]
    }

    #[test]
    fn scalars_are_ffi_safe_at_every_position() {
        let tcx = TyCtxt::new();
        for kind in scalar_kinds() {
            let ty = tcx.intern(kind);
            for position in [
                FfiPosition::Parameter,
                FfiPosition::Return,
                FfiPosition::Static,
                FfiPosition::Field,
                FfiPosition::VariadicArgument,
                FfiPosition::FunctionPointer,
            ] {
                assert_eq!(
                    safety(&tcx, ty, position),
                    Ok(()),
                    "{kind:?} at {position:?}"
                );
            }
        }
    }

    #[test]
    fn unit_is_a_return_type_only() {
        let tcx = TyCtxt::new();
        let unit = tcx.unit_ty();
        assert_eq!(safety(&tcx, unit, FfiPosition::Return), Ok(()));
        assert_eq!(
            safety(&tcx, unit, FfiPosition::Parameter),
            Err(NotFfiSafe::UnitParameter)
        );
        assert_eq!(
            safety(&tcx, unit, FfiPosition::Field),
            Err(NotFfiSafe::UnitParameter)
        );
    }

    #[test]
    fn unsized_types_and_fat_pointers_are_rejected() {
        let tcx = TyCtxt::new();
        let string = tcx.str_ty();
        let slice = tcx.slice_ty(tcx.int_ty(crate::IntTy::I32));
        assert_eq!(
            safety(&tcx, string, FfiPosition::Parameter),
            Err(NotFfiSafe::Unsized)
        );
        assert_eq!(
            safety(&tcx, slice, FfiPosition::Return),
            Err(NotFfiSafe::Unsized)
        );
        for fat in [string, slice] {
            let pointer = tcx.raw_ptr_ty_with(RawPtrKind::Const, fat);
            assert_eq!(
                safety(&tcx, pointer, FfiPosition::Parameter),
                Err(NotFfiSafe::FatPointer)
            );
        }
    }

    #[test]
    fn thin_raw_pointers_are_safe_and_references_are_not() {
        let tcx = TyCtxt::new();
        let int = tcx.int_ty(crate::IntTy::I32);
        let pointer = tcx.raw_ptr_ty_with(RawPtrKind::Const, int);
        assert_eq!(safety(&tcx, pointer, FfiPosition::Parameter), Ok(()));

        for kind in [RefKind::Shared, RefKind::Exclusive] {
            let reference = tcx.ref_ty_with(kind, int);
            assert_eq!(
                safety(&tcx, reference, FfiPosition::Parameter),
                Err(NotFfiSafe::Reference(kind))
            );
        }
    }

    #[test]
    fn variadic_promotions_follow_the_c_defaults() {
        let tcx = TyCtxt::new();
        let int = tcx.int_ty(crate::IntTy::I32);
        let float = tcx.float_ty(crate::FloatTy::F64);
        let pointer = tcx.raw_ptr_ty_with(RawPtrKind::Const, int);

        for narrow in [
            tcx.bool_ty(),
            tcx.int_ty(crate::IntTy::I8),
            tcx.int_ty(crate::IntTy::I16),
            tcx.uint_ty(crate::UintTy::U8),
            tcx.uint_ty(crate::UintTy::U16),
        ] {
            assert_eq!(tcx.variadic_promotion(narrow), Some(int));
        }
        assert_eq!(
            tcx.variadic_promotion(tcx.float_ty(crate::FloatTy::F32)),
            Some(float)
        );
        for unchanged in [
            int,
            tcx.uint_ty(crate::UintTy::U32),
            tcx.int_ty(crate::IntTy::I64),
            tcx.uint_ty(crate::UintTy::U64),
            float,
            pointer,
        ] {
            assert_eq!(tcx.variadic_promotion(unchanged), Some(unchanged));
        }
    }

    #[test]
    fn types_cannot_cross_a_variadic_boundary() {
        let tcx = TyCtxt::new();
        let int = tcx.int_ty(crate::IntTy::I32);
        for rejected in [
            tcx.unit_ty(),
            tcx.char_ty(),
            tcx.str_ty(),
            tcx.slice_ty(int),
            tcx.ref_ty_with(RefKind::Shared, int),
            tcx.int_ty(crate::IntTy::I128),
            tcx.uint_ty(crate::UintTy::U128),
        ] {
            assert_eq!(tcx.variadic_promotion(rejected), None);
        }
        let fat = tcx.raw_ptr_ty_with(RawPtrKind::Const, tcx.str_ty());
        assert_eq!(tcx.variadic_promotion(fat), None);
    }

    #[test]
    fn aggregates_tuples_chars_and_wide_integers_are_rejected() {
        let tcx = TyCtxt::new();
        let int = tcx.int_ty(crate::IntTy::I32);
        let tuple = tcx.tuple_ty(&[int, int]);
        assert_eq!(
            safety(&tcx, tuple, FfiPosition::Parameter),
            Err(NotFfiSafe::Tuple)
        );
        assert_eq!(
            safety(&tcx, tcx.char_ty(), FfiPosition::Parameter),
            Err(NotFfiSafe::Char)
        );
        for wide in [
            tcx.int_ty(crate::IntTy::I128),
            tcx.uint_ty(crate::UintTy::U128),
        ] {
            for position in [
                FfiPosition::Parameter,
                FfiPosition::Return,
                FfiPosition::Field,
            ] {
                assert_eq!(safety(&tcx, wide, position), Err(NotFfiSafe::WideInteger));
            }
        }
    }

    #[test]
    fn zero_length_arrays_are_rejected_and_non_empty_arrays_recurse() {
        let tcx = TyCtxt::new();
        let int = tcx.int_ty(crate::IntTy::I32);
        let empty = tcx.array_ty(int, 0);
        assert_eq!(
            safety(&tcx, empty, FfiPosition::Field),
            Err(NotFfiSafe::EmptyArray)
        );
        let non_empty = tcx.array_ty(int, 4);
        assert_eq!(safety(&tcx, non_empty, FfiPosition::Field), Ok(()));
        let characters = tcx.array_ty(tcx.char_ty(), 4);
        assert_eq!(
            safety(&tcx, characters, FfiPosition::Field),
            Err(NotFfiSafe::Char)
        );
    }
}
