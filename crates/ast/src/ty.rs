//! Type syntax.

use seal_span::Span;
use seal_symbol::Symbol;

use crate::id::TypeId;

/// A built-in type name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PrimitiveType {
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
    /// `f32`
    F32,
    /// `f64`
    F64,
    /// `bool`
    Bool,
    /// `char`
    Char,
    /// `str`
    Str,
}

impl PrimitiveType {
    /// Every primitive type, in declaration order.
    pub const ALL: [Self; 17] = [
        Self::I8,
        Self::I16,
        Self::I32,
        Self::I64,
        Self::I128,
        Self::Isize,
        Self::U8,
        Self::U16,
        Self::U32,
        Self::U64,
        Self::U128,
        Self::Usize,
        Self::F32,
        Self::F64,
        Self::Bool,
        Self::Char,
        Self::Str,
    ];

    /// Parses a primitive type by its source spelling.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|primitive| primitive.as_str() == name)
    }

    /// Returns the source spelling of the type.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::I8 => "i8",
            Self::I16 => "i16",
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::I128 => "i128",
            Self::Isize => "isize",
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
            Self::U64 => "u64",
            Self::U128 => "u128",
            Self::Usize => "usize",
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::Bool => "bool",
            Self::Char => "char",
            Self::Str => "str",
        }
    }
}

/// A type as written in the source.
#[derive(Clone, Debug, PartialEq)]
pub struct Type {
    /// What the type is.
    pub kind: TypeKind,
    /// The type's source range.
    pub span: Span,
}

/// The kinds of type.
#[derive(Clone, Debug, PartialEq)]
pub enum TypeKind {
    /// A built-in type.
    Primitive(PrimitiveType),
    /// A reference to another type, written `*T` or `*mut T`.
    Pointer {
        /// The referenced type.
        pointee: TypeId,
        /// `true` for `*mut T`, which permits writing through the reference.
        mutable: bool,
    },
    /// A named type, resolved against the type namespace during lowering.
    Path(Symbol),
    /// A fixed-size array, written `[T; N]`.
    Array {
        /// The type of each element.
        element: TypeId,
        /// The number of elements.
        length: u64,
        /// The span of the length literal, for diagnostics.
        length_span: Span,
    },
    /// An unsized slice, written `[T]`.
    Slice {
        /// The type of each element.
        element: TypeId,
    },
    /// Syntax the parser could not turn into a type.
    Error,
}
