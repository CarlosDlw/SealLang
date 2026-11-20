//! Type syntax.

use seal_span::Span;

use crate::generics::{TraitPath, TypePath};
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
    /// `c_char`
    CChar,
    /// `c_schar`
    CSchar,
    /// `c_uchar`
    CUchar,
    /// `c_short`
    CShort,
    /// `c_ushort`
    CUshort,
    /// `c_int`
    CInt,
    /// `c_uint`
    CUint,
    /// `c_long`
    CLong,
    /// `c_ulong`
    CUlong,
    /// `c_longlong`
    CLongLong,
    /// `c_ulonglong`
    CUlongLong,
    /// `c_float`
    CFloat,
    /// `c_double`
    CDouble,
    /// `c_bool`
    CBool,
    /// `c_wchar`
    CWchar,
    /// `size_t`
    SizeT,
    /// `ssize_t`
    SsizeT,
    /// `ptrdiff_t`
    PtrdiffT,
    /// `intptr_t`
    IntptrT,
    /// `uintptr_t`
    UintptrT,
}

impl PrimitiveType {
    /// Every primitive type, in declaration order.
    pub const ALL: [Self; 37] = [
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
        Self::CChar,
        Self::CSchar,
        Self::CUchar,
        Self::CShort,
        Self::CUshort,
        Self::CInt,
        Self::CUint,
        Self::CLong,
        Self::CUlong,
        Self::CLongLong,
        Self::CUlongLong,
        Self::CFloat,
        Self::CDouble,
        Self::CBool,
        Self::CWchar,
        Self::SizeT,
        Self::SsizeT,
        Self::PtrdiffT,
        Self::IntptrT,
        Self::UintptrT,
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
            Self::CChar => "c_char",
            Self::CSchar => "c_schar",
            Self::CUchar => "c_uchar",
            Self::CShort => "c_short",
            Self::CUshort => "c_ushort",
            Self::CInt => "c_int",
            Self::CUint => "c_uint",
            Self::CLong => "c_long",
            Self::CUlong => "c_ulong",
            Self::CLongLong => "c_longlong",
            Self::CUlongLong => "c_ulonglong",
            Self::CFloat => "c_float",
            Self::CDouble => "c_double",
            Self::CBool => "c_bool",
            Self::CWchar => "c_wchar",
            Self::SizeT => "size_t",
            Self::SsizeT => "ssize_t",
            Self::PtrdiffT => "ptrdiff_t",
            Self::IntptrT => "intptr_t",
            Self::UintptrT => "uintptr_t",
        }
    }

    /// Returns whether this name may suffix a numeric literal.
    ///
    /// The C scalar names are type names only: `1c_int` is not a literal.
    pub const fn is_literal_suffix(self) -> bool {
        matches!(
            self,
            Self::I8
                | Self::I16
                | Self::I32
                | Self::I64
                | Self::I128
                | Self::Isize
                | Self::U8
                | Self::U16
                | Self::U32
                | Self::U64
                | Self::U128
                | Self::Usize
                | Self::F32
                | Self::F64
        )
    }
}

#[cfg(test)]
mod tests {
    use super::PrimitiveType;

    #[test]
    fn c_scalar_names_are_primitives() {
        for name in [
            "c_char",
            "c_schar",
            "c_uchar",
            "c_short",
            "c_ushort",
            "c_int",
            "c_uint",
            "c_long",
            "c_ulong",
            "c_longlong",
            "c_ulonglong",
            "c_float",
            "c_double",
            "c_bool",
            "c_wchar",
            "size_t",
            "ssize_t",
            "ptrdiff_t",
            "intptr_t",
            "uintptr_t",
        ] {
            let primitive = PrimitiveType::from_name(name).expect("the C name is a primitive");
            assert_eq!(primitive.as_str(), name);
        }
    }

    #[test]
    fn only_numeric_primitives_suffix_a_literal() {
        assert!(PrimitiveType::I32.is_literal_suffix());
        assert!(PrimitiveType::Usize.is_literal_suffix());
        assert!(PrimitiveType::F64.is_literal_suffix());
        assert!(!PrimitiveType::CInt.is_literal_suffix());
        assert!(!PrimitiveType::CBool.is_literal_suffix());
        assert!(!PrimitiveType::Bool.is_literal_suffix());
        assert!(!PrimitiveType::Str.is_literal_suffix());
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
    /// A raw pointer, written `*raw T` or `*raw mut T`.
    ///
    /// Raw pointers are nullable and support arithmetic; they are the unsafe
    /// layer's pointer type and never unify with a reference.
    RawPointer {
        /// The pointed-to type.
        pointee: TypeId,
        /// `true` for `*raw mut T`, which permits writing through the pointer.
        mutable: bool,
    },
    /// A named type, resolved against the type namespace during lowering.
    ///
    /// The optional argument list is kept here so nested generic types and
    /// `Self` arguments retain their exact source spans.
    Path(TypePath),
    /// A trait object, written `dyn Trait` or `dyn Trait<Args>`.
    ///
    /// Trait objects are unsized and only become usable behind a pointer. The
    /// path is kept unresolved here; HIR validates that it names a trait and
    /// applies the dyn-compatibility rules.
    Dyn(TraitPath),
    /// A tuple type, written `(A, B, ...)`.
    ///
    /// The empty list is the unit type `()`. A one-element tuple must keep its
    /// trailing comma, so `(A,)` is a tuple while `(A)` is just a parenthesized
    /// type.
    Tuple(Vec<TypeId>),
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
    /// A Seal function pointer type, written `func(A, B) -> R` or
    /// `unsafe func(A, B) -> R`.
    ///
    /// This is a first-class value type with the internal Seal ABI. It is
    /// distinct from [`Self::FnSig`], which only describes a C function behind
    /// a raw pointer.
    FnPtr {
        /// Fixed parameter types in declaration order.
        params: Vec<TypeId>,
        /// The return type, or `None` when the source omits `-> T`.
        ret: Option<TypeId>,
        /// `true` for `unsafe func(...)`, which requires `unsafe` to call.
        is_unsafe: bool,
    },
    /// A C function signature, written `extern "c" func(T, ...) -> R`.
    ///
    /// The signature only appears as the pointee of a raw pointer; the parser
    /// produces it after `*raw extern`. It is never a value type.
    FnSig {
        /// The ABI spelling written between the quotes.
        abi: seal_symbol::Symbol,
        /// The span of the ABI string, for diagnostics.
        abi_span: Span,
        /// Fixed parameter types in declaration order.
        params: Vec<TypeId>,
        /// The return type, or `None` when the source omits `-> T`.
        ret: Option<TypeId>,
        /// Whether the parameter list ends with `...`.
        variadic: bool,
        /// The span of `...`, when present.
        variadic_span: Option<Span>,
    },
    /// Syntax the parser could not turn into a type.
    Error,
}
