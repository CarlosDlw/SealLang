//! The kinds of semantic types and the scalar families they use.

use std::fmt;

use crate::id::{InferTyId, TyId};

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

/// The meaning of a type.
///
/// The variants mirror the language surface: scalars, the unsized `str`, shared
/// references, the unit and never types, and the two non-user types (`Error` and
/// `Infer`). Aggregates are added when the language gains them.
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
    /// A signed integer type.
    Int(IntTy),
    /// An unsigned integer type.
    Uint(UintTy),
    /// A floating-point type.
    Float(FloatTy),
    /// A shared, non-owning reference, written `*T`.
    Ref(TyId),
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
    use super::{FloatTy, IntTy, UintTy};

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
}
