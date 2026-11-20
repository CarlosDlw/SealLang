//! The kinds of semantic types and the scalar families they use.

use std::fmt;

use seal_def::DefId;
use seal_symbol::Symbol;

use crate::id::{AdtId, FnSigId, InferTyId, ParamTyId, SubstsId, TupleId, TyId};

/// A signed integer type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum IntTy {
    /// `i8`
    I8,
    /// `i16`
    I16,
    /// `i32`
    I32,
    /// `i64`
    I64,
    /// `i128`
    I128,
    /// `isize`
    Isize,
}

impl IntTy {
    /// Every signed integer type, in increasing width.
    pub const ALL: [Self; 6] = [
        Self::I8,
        Self::I16,
        Self::I32,
        Self::I64,
        Self::I128,
        Self::Isize,
    ];

    /// Returns the source spelling of the type.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::I128 => "i128",
            Self::Isize => "isize",
        }
    }

    /// Returns the width in bits, or `None` for the pointer-sized `isize`.
    pub const fn bits(self) -> Option<u16> {
        match self {
            Self::I8 => Some(8),
            Self::I16 => Some(16),
            Self::I32 => Some(32),
            Self::I64 => Some(64),
            Self::I128 => Some(128),
            Self::Isize => None,
        }
    }
}

impl fmt::Display for IntTy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An unsigned integer type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum UintTy {
    /// `u8`
    U8,
    /// `u16`
    U16,
    /// `u32`
    U32,
    /// `u64`
    U64,
    /// `u128`
    U128,
    /// `usize`
    Usize,
}

impl UintTy {
    /// Every unsigned integer type, in increasing width.
    pub const ALL: [Self; 6] = [
        Self::U8,
        Self::U16,
        Self::U32,
        Self::U64,
        Self::U128,
        Self::Usize,
    ];

    /// Returns the source spelling of the type.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::U128 => "u128",
            Self::Usize => "usize",
        }
    }

    /// Returns the width in bits, or `None` for the pointer-sized `usize`.
    pub const fn bits(self) -> Option<u16> {
        match self {
            Self::U8 => Some(8),
            Self::U16 => Some(16),
            Self::U32 => Some(32),
            Self::U64 => Some(64),
            Self::U128 => Some(128),
            Self::Usize => None,
        }
    }
}

impl fmt::Display for UintTy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A floating-point type.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FloatTy {
    /// `f32`
    F32,
    /// `f64`
    F64,
}

impl FloatTy {
    /// Every floating-point type, in increasing width.
    pub const ALL: [Self; 2] = [Self::F32, Self::F64];

    /// Returns the source spelling of the type.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::F64 => "f64",
        }
    }

    /// Returns the width in bits.
    pub const fn bits(self) -> u16 {
        match self {
            Self::F32 => 32,
            Self::F64 => 64,
        }
    }
}

impl fmt::Display for FloatTy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether a reference permits writing through it.
///
/// The kind is part of the type, so `*i32` and `*mut i32` are distinct types and
/// never unify. Decision D4 of the memory model gives `*T` shared, read-only
/// access; decision D6 makes `*mut T` exclusive and read-write.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RefKind {
    /// `*T`: shared and read-only; any number of live shared loans may overlap.
    Shared,
    /// `*mut T`: exclusive and read-write; no other loan may overlap.
    Exclusive,
}

impl RefKind {
    /// Returns the prefix that names the reference kind in the source.
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Shared => "*",
            Self::Exclusive => "*mut ",
        }
    }

    /// Returns `true` for `*mut T`.
    pub const fn is_exclusive(self) -> bool {
        matches!(self, Self::Exclusive)
    }
}

impl fmt::Display for RefKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.prefix())
    }
}

/// Whether a raw pointer permits writing through it.
///
/// Raw pointers are the unsafe layer's pointers: nullable, arithmetic, and
/// distinct from references. `*raw T` is read-only; `*raw mut T` is writable.
/// The kind is part of the type, so the two never unify.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RawPtrKind {
    /// `*raw T`: read-only.
    Const,
    /// `*raw mut T`: writable.
    Mut,
}

impl RawPtrKind {
    /// Returns the prefix that names the kind in the source.
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Const => "*raw ",
            Self::Mut => "*raw mut ",
        }
    }

    /// Returns `true` for `*raw mut T`.
    pub const fn is_mut(self) -> bool {
        matches!(self, Self::Mut)
    }
}

impl fmt::Display for RawPtrKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.prefix())
    }
}

/// The meaning of a type.
///
/// The variants mirror the language surface: scalars, the unsized `str` and
/// slices, references, raw pointers, nominal aggregates, the unit and never
/// types, and the two non-user types (`Error` and `Infer`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TyKind {
    /// The boolean type `bool`.
    Bool,
    /// A Unicode scalar value, `char`.
    Char,
    /// The unsized string pointee, `str`.
    ///
    /// Values of this kind only appear behind a reference; see the memory model.
    Str,
    /// The unsized slice pointee, `[T]`.
    ///
    /// A slice is a view over a contiguous sequence of `element` values. It is
    /// never stored by value: `*[T]` and `*mut [T]` are its fat-pointer
    /// references, carrying the element address and the length.
    Slice {
        /// The type of every element.
        element: TyId,
    },
    /// A signed integer type.
    Int(IntTy),
    /// An unsigned integer type.
    Uint(UintTy),
    /// A floating-point type.
    Float(FloatTy),
    /// A non-owning reference, written `*T` or `*mut T`.
    Ref {
        /// Whether the reference permits writing through it.
        kind: RefKind,
        /// The referenced type.
        pointee: TyId,
    },
    /// A raw pointer, written `*raw T` or `*raw mut T`.
    ///
    /// Unlike a reference, a raw pointer is nullable, supports arithmetic, and
    /// is only dereferenced inside `unsafe`; see decisions D21 and D22 of the
    /// memory model.
    RawPtr {
        /// Whether the pointer permits writing through it.
        kind: RawPtrKind,
        /// The pointed-to type.
        pointee: TyId,
    },
    /// A fixed-size array `[T; N]`.
    Array {
        /// The type of every element.
        element: TyId,
        /// The number of elements.
        length: u64,
    },
    /// An opaque type with no size, alignment, fields, or destructor.
    ///
    /// Declared by `extern type Name;` or named by the built-in `c_void`. The
    /// type only appears behind a raw pointer; `C` owns the storage and its
    /// layout, so Seal never computes either.
    Opaque {
        /// The definition that introduced the type, for diagnostics and identity.
        def: DefId,
        /// The declared name, used by type display.
        name: Symbol,
    },
    /// A nominal aggregate type with its type arguments, such as `Pair<i32, bool>`.
    ///
    /// The [`AdtId`] names the declaration, while `args` supplies one type for
    /// each declared parameter. A non-generic aggregate uses the canonical empty
    /// substitution. Two instantiations with different arguments are different
    /// types and therefore have different `TyId`s.
    Adt {
        /// The aggregate declaration.
        adt: AdtId,
        /// The ordered type arguments, one per declared parameter.
        args: SubstsId,
    },
    /// A declared type parameter used as a type, such as `T`.
    ///
    /// Parameters are rigid during checking of a generic body. They are replaced
    /// by [`TyCtxt::substitute`](crate::TyCtxt::substitute) when a declaration is
    /// instantiated.
    Param(ParamTyId),
    /// A trait object, written `dyn Trait` or `dyn Trait<Args>`.
    ///
    /// The object is unsized. Its identity is the trait declaration plus the
    /// trait's own type arguments; the concrete implementing type is erased and
    /// only appears behind a fat pointer such as `*dyn Trait`.
    Dyn {
        /// The principal trait declaration.
        trait_def: DefId,
        /// The trait's own type arguments, excluding `Self`.
        args: SubstsId,
    },
    /// A structural tuple type `(T, U, ...)`.
    ///
    /// The tuple's element types live in [`TyCtxt`](crate::TyCtxt)'s tuple
    /// arena. The empty tuple is represented as [`TyKind::Unit`], not as a
    /// zero-element tuple, so `()` has one canonical meaning everywhere.
    Tuple(TupleId),
    /// A C function signature, written `extern "c" func(...) -> ...`.
    ///
    /// The signature is unsized and only appears as the pointee of a raw
    /// pointer; `*raw extern "c" func(...)` is the callable C function pointer.
    /// Its parameter and return types live in the function-signature arena.
    FnSig(FnSigId),
    /// The unit type `()`.
    Unit,
    /// The never type `!`, the type of expressions that do not return.
    Never,
    /// A type that already produced a diagnostic.
    ///
    /// Creating this kind without having reported an error would suppress later
    /// errors, so it is only produced during error recovery.
    Error,
    /// An inference variable that is not known yet.
    Infer(InferTyId),
}

#[cfg(test)]
mod tests {
    use super::{FloatTy, IntTy, RawPtrKind, RefKind, UintTy};

    #[test]
    fn every_int_type_has_a_distinct_spelling() {
        let mut names: Vec<&str> = IntTy::ALL.iter().map(|ty| ty.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), IntTy::ALL.len());
    }

    #[test]
    fn every_uint_type_has_a_distinct_spelling() {
        let mut names: Vec<&str> = UintTy::ALL.iter().map(|ty| ty.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), UintTy::ALL.len());
    }

    #[test]
    fn int_widths_are_ordered() {
        let widths: Vec<Option<u16>> = IntTy::ALL.iter().map(|ty| ty.bits()).collect();
        assert_eq!(
            widths,
            vec![Some(8), Some(16), Some(32), Some(64), Some(128), None]
        );
        assert_eq!(IntTy::Isize.bits(), None);
        assert_eq!(IntTy::I32.to_string(), "i32");
    }

    #[test]
    fn uint_widths_are_ordered() {
        let widths: Vec<Option<u16>> = UintTy::ALL.iter().map(|ty| ty.bits()).collect();
        assert_eq!(
            widths,
            vec![Some(8), Some(16), Some(32), Some(64), Some(128), None]
        );
        assert_eq!(UintTy::Usize.bits(), None);
        assert_eq!(UintTy::U64.to_string(), "u64");
    }

    #[test]
    fn float_widths_match_their_names() {
        assert_eq!(FloatTy::F32.bits(), 32);
        assert_eq!(FloatTy::F64.bits(), 64);
        assert_eq!(FloatTy::F32.to_string(), "f32");
        assert!(FloatTy::F32 != FloatTy::F64);
    }

    #[test]
    fn reference_kinds_have_distinct_spellings() {
        assert_eq!(RefKind::Shared.prefix(), "*");
        assert_eq!(RefKind::Exclusive.prefix(), "*mut ");
        assert_eq!(RefKind::Exclusive.to_string(), "*mut ");
        assert!(RefKind::Exclusive.is_exclusive());
        assert!(!RefKind::Shared.is_exclusive());
        assert_ne!(RefKind::Shared, RefKind::Exclusive);
    }

    #[test]
    fn raw_pointer_kinds_have_distinct_spellings() {
        assert_eq!(RawPtrKind::Const.prefix(), "*raw ");
        assert_eq!(RawPtrKind::Mut.prefix(), "*raw mut ");
        assert_eq!(RawPtrKind::Mut.to_string(), "*raw mut ");
        assert!(RawPtrKind::Mut.is_mut());
        assert!(!RawPtrKind::Const.is_mut());
        assert_ne!(RawPtrKind::Const, RawPtrKind::Mut);
    }
}
