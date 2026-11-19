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

use crate::{IntTy, RefKind, TyCtxt, TyId, TyKind, UintTy};

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
            Self::EmptyArray => "use a non-empty array",
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
        TyKind::Adt { .. } => Err(NotFfiSafe::Aggregate),
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
