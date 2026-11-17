//! The compiler-owned intrinsic method catalog.
//!
//! Seal methods are resolved statically. This crate is the single source of
//! truth for their names, owners, receiver modes, signatures, and lowering
//! operations; type checking, MIR, and code generation all consume the same
//! entries, so a method cannot mean different things in different phases.
//!
//! The catalog is closed in v0.1: users cannot add methods to primitive types,
//! arrays, or slices. Methods on nominal structs arrive with `impl`, which will
//! add a separate user-method table without changing this lookup.

#![warn(missing_docs)]

/// A stable identifier for one entry in [`METHODS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MethodId(u16);

impl MethodId {
    /// Creates an identifier from its catalog index.
    ///
    /// # Panics
    ///
    /// Panics if `index` does not name an entry. Catalog identifiers are an
    /// internal compiler value, never user input.
    pub const fn from_index(index: usize) -> Self {
        assert!(index < METHODS.len(), "method index is out of range");
        Self(index as u16)
    }

    /// Returns the catalog index.
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Returns the definition for this method.
    pub fn def(self) -> &'static MethodDef {
        &METHODS[self.index()]
    }
}

/// The family of types a method belongs to.
///
/// Integer and float methods are shared by every width in the family; the
/// receiver type instantiates `Self`. Arrays and slices are distinct owners so
/// their storage models never become confused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MethodOwner {
    /// Signed integer types.
    Int,
    /// Unsigned integer types.
    Uint,
    /// Floating-point types.
    Float,
    /// `bool`.
    Bool,
    /// `char`.
    Char,
    /// Fixed-size arrays `[T; N]`.
    Array,
    /// Slice pointees `[T]`, reached through `*[T]` or `*mut [T]`.
    Slice,
}

impl MethodOwner {
    /// Returns the source-level name of the owner family.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Int => "signed integer",
            Self::Uint => "unsigned integer",
            Self::Float => "floating-point",
            Self::Bool => "bool",
            Self::Char => "char",
            Self::Array => "array",
            Self::Slice => "slice",
        }
    }
}

/// How a method receives its receiver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiverMode {
    /// Takes the receiver by value. The receiver must be `Copy`; all scalar
    /// owners in the catalog are.
    Value,
    /// Reads through a shared reference.
    Shared,
    /// Reads and writes through an exclusive reference.
    Exclusive,
}

/// One parameter type in a method signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamKind {
    /// The receiver's base type: the scalar type, or the array/slice element
    /// type for `fill`.
    SelfValue,
    /// The array or slice element type.
    Element,
    /// `usize`.
    Usize,
    /// `i32`.
    I32,
    /// `u32`.
    U32,
}

/// The result type of a method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReturnKind {
    /// The receiver's base type.
    Same,
    /// `bool`.
    Bool,
    /// `usize`.
    Usize,
    /// `i32`.
    I32,
    /// `u32`.
    U32,
    /// The unit type.
    Unit,
}

/// A compiler operation behind a method.
///
/// The variants are intentionally semantic: code generation chooses the LLVM
/// instruction, intrinsic, or loop that implements the documented behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntrinsicOp {
    /// Signed integer absolute value, wrapping at `MIN`.
    IntAbs,
    /// Signed integer sign: -1, 0, or 1.
    IntSignum,
    /// Smaller of two integers.
    IntMin,
    /// Larger of two integers.
    IntMax,
    /// Clamp an integer between two bounds.
    IntClamp,
    /// Wrapping integer addition.
    IntWrappingAdd,
    /// Wrapping integer subtraction.
    IntWrappingSub,
    /// Wrapping integer multiplication.
    IntWrappingMul,
    /// Saturating integer addition.
    IntSaturatingAdd,
    /// Saturating integer subtraction.
    IntSaturatingSub,
    /// Saturating integer multiplication.
    IntSaturatingMul,
    /// Number of set bits.
    IntCountOnes,
    /// Number of leading zero bits.
    IntLeadingZeros,
    /// Number of trailing zero bits.
    IntTrailingZeros,
    /// Rotate left by a `u32` amount.
    IntRotateLeft,
    /// Rotate right by a `u32` amount.
    IntRotateRight,
    /// Reverse the bytes of the integer.
    IntSwapBytes,
    /// Floating-point absolute value.
    FloatAbs,
    /// Smaller floating-point value, choosing the non-NaN operand.
    FloatMin,
    /// Larger floating-point value, choosing the non-NaN operand.
    FloatMax,
    /// Clamp a float between two bounds.
    FloatClamp,
    /// Whether the value is NaN.
    FloatIsNan,
    /// Whether the value is finite.
    FloatIsFinite,
    /// Whether the value is an infinity.
    FloatIsInfinite,
    /// Square root.
    FloatSqrt,
    /// Round toward negative infinity.
    FloatFloor,
    /// Round toward positive infinity.
    FloatCeil,
    /// Round to the nearest integer, halfway away from zero.
    FloatRound,
    /// Round toward zero.
    FloatTrunc,
    /// Fractional part, `self - trunc(self)`.
    FloatFract,
    /// Raise to an integer power.
    FloatPowi,
    /// Boolean negation.
    BoolNot,
    /// Convert `false`/`true` to `0`/`1`.
    BoolToI32,
    /// Negate a mutable boolean in place.
    BoolToggle,
    /// Whether a char is ASCII.
    CharIsAscii,
    /// Whether a char is an ASCII letter.
    CharIsAsciiAlphabetic,
    /// Whether a char is an ASCII digit.
    CharIsAsciiDigit,
    /// Whether a char is ASCII whitespace.
    CharIsAsciiWhitespace,
    /// Uppercase an ASCII letter, leaving other chars unchanged.
    CharToAsciiUppercase,
    /// Lowercase an ASCII letter, leaving other chars unchanged.
    CharToAsciiLowercase,
    /// Number of bytes in the UTF-8 encoding.
    CharLenUtf8,
    /// The Unicode scalar value as `u32`.
    CharToU32,
    /// Number of elements in an array or slice.
    Len,
    /// Whether an array or slice has no elements.
    IsEmpty,
    /// Swap two elements by index, panicking when either index is out of range.
    Swap,
    /// Reverse all elements in place.
    Reverse,
    /// Copy one value into every element; the type checker requires `Copy`.
    Fill,
}

/// One compiler-provided method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MethodDef {
    /// The type family this method belongs to.
    pub owner: MethodOwner,
    /// The source-level method name.
    pub name: &'static str,
    /// How the receiver is passed.
    pub receiver: ReceiverMode,
    /// Argument types after the receiver.
    pub params: &'static [ParamKind],
    /// Result type.
    pub ret: ReturnKind,
    /// The operation that implements the method.
    pub intrinsic: IntrinsicOp,
    /// Whether `fill` requires `Copy` on the element type.
    pub requires_copy_element: bool,
}

const NONE: &[ParamKind] = &[];
const SELF_ONE: &[ParamKind] = &[ParamKind::SelfValue];
const SELF_TWO: &[ParamKind] = &[ParamKind::SelfValue, ParamKind::SelfValue];
const USIZE_TWO: &[ParamKind] = &[ParamKind::Usize, ParamKind::Usize];
const U32_ONE: &[ParamKind] = &[ParamKind::U32];
const ELEMENT_ONE: &[ParamKind] = &[ParamKind::Element];

const fn method(
    owner: MethodOwner,
    name: &'static str,
    receiver: ReceiverMode,
    params: &'static [ParamKind],
    ret: ReturnKind,
    intrinsic: IntrinsicOp,
) -> MethodDef {
    MethodDef {
        owner,
        name,
        receiver,
        params,
        ret,
        intrinsic,
        requires_copy_element: false,
    }
}

const fn fill(owner: MethodOwner) -> MethodDef {
    MethodDef {
        owner,
        name: "fill",
        receiver: ReceiverMode::Exclusive,
        params: ELEMENT_ONE,
        ret: ReturnKind::Unit,
        intrinsic: IntrinsicOp::Fill,
        requires_copy_element: true,
    }
}

/// Every compiler-owned method, in stable identifier order.
pub const METHODS: &[MethodDef] = &[
    method(
        MethodOwner::Int,
        "abs",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Same,
        IntrinsicOp::IntAbs,
    ),
    method(
        MethodOwner::Int,
        "signum",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Same,
        IntrinsicOp::IntSignum,
    ),
    method(
        MethodOwner::Int,
        "min",
        ReceiverMode::Value,
        SELF_ONE,
        ReturnKind::Same,
        IntrinsicOp::IntMin,
    ),
    method(
        MethodOwner::Int,
        "max",
        ReceiverMode::Value,
        SELF_ONE,
        ReturnKind::Same,
        IntrinsicOp::IntMax,
    ),
    method(
        MethodOwner::Int,
        "clamp",
        ReceiverMode::Value,
        SELF_TWO,
        ReturnKind::Same,
        IntrinsicOp::IntClamp,
    ),
    method(
        MethodOwner::Int,
        "wrapping_add",
        ReceiverMode::Value,
        SELF_ONE,
        ReturnKind::Same,
        IntrinsicOp::IntWrappingAdd,
    ),
    method(
        MethodOwner::Int,
        "wrapping_sub",
        ReceiverMode::Value,
        SELF_ONE,
        ReturnKind::Same,
        IntrinsicOp::IntWrappingSub,
    ),
    method(
        MethodOwner::Int,
        "wrapping_mul",
        ReceiverMode::Value,
        SELF_ONE,
        ReturnKind::Same,
        IntrinsicOp::IntWrappingMul,
    ),
    method(
        MethodOwner::Int,
        "saturating_add",
        ReceiverMode::Value,
        SELF_ONE,
        ReturnKind::Same,
        IntrinsicOp::IntSaturatingAdd,
    ),
    method(
        MethodOwner::Int,
        "saturating_sub",
        ReceiverMode::Value,
        SELF_ONE,
        ReturnKind::Same,
        IntrinsicOp::IntSaturatingSub,
    ),
    method(
        MethodOwner::Int,
        "saturating_mul",
        ReceiverMode::Value,
        SELF_ONE,
        ReturnKind::Same,
        IntrinsicOp::IntSaturatingMul,
    ),
    method(
        MethodOwner::Int,
        "count_ones",
        ReceiverMode::Value,
        NONE,
        ReturnKind::U32,
        IntrinsicOp::IntCountOnes,
    ),
    method(
        MethodOwner::Int,
        "leading_zeros",
        ReceiverMode::Value,
        NONE,
        ReturnKind::U32,
        IntrinsicOp::IntLeadingZeros,
    ),
    method(
        MethodOwner::Int,
        "trailing_zeros",
        ReceiverMode::Value,
        NONE,
        ReturnKind::U32,
        IntrinsicOp::IntTrailingZeros,
    ),
    method(
        MethodOwner::Int,
        "rotate_left",
        ReceiverMode::Value,
        U32_ONE,
        ReturnKind::Same,
        IntrinsicOp::IntRotateLeft,
    ),
    method(
        MethodOwner::Int,
        "rotate_right",
        ReceiverMode::Value,
        U32_ONE,
        ReturnKind::Same,
        IntrinsicOp::IntRotateRight,
    ),
    method(
        MethodOwner::Int,
        "swap_bytes",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Same,
        IntrinsicOp::IntSwapBytes,
    ),
    method(
        MethodOwner::Uint,
        "min",
        ReceiverMode::Value,
        SELF_ONE,
        ReturnKind::Same,
        IntrinsicOp::IntMin,
    ),
    method(
        MethodOwner::Uint,
        "max",
        ReceiverMode::Value,
        SELF_ONE,
        ReturnKind::Same,
        IntrinsicOp::IntMax,
    ),
    method(
        MethodOwner::Uint,
        "clamp",
        ReceiverMode::Value,
        SELF_TWO,
        ReturnKind::Same,
        IntrinsicOp::IntClamp,
    ),
    method(
        MethodOwner::Uint,
        "wrapping_add",
        ReceiverMode::Value,
        SELF_ONE,
        ReturnKind::Same,
        IntrinsicOp::IntWrappingAdd,
    ),
    method(
        MethodOwner::Uint,
        "wrapping_sub",
        ReceiverMode::Value,
        SELF_ONE,
        ReturnKind::Same,
        IntrinsicOp::IntWrappingSub,
    ),
    method(
        MethodOwner::Uint,
        "wrapping_mul",
        ReceiverMode::Value,
        SELF_ONE,
        ReturnKind::Same,
        IntrinsicOp::IntWrappingMul,
    ),
    method(
        MethodOwner::Uint,
        "saturating_add",
        ReceiverMode::Value,
        SELF_ONE,
        ReturnKind::Same,
        IntrinsicOp::IntSaturatingAdd,
    ),
    method(
        MethodOwner::Uint,
        "saturating_sub",
        ReceiverMode::Value,
        SELF_ONE,
        ReturnKind::Same,
        IntrinsicOp::IntSaturatingSub,
    ),
    method(
        MethodOwner::Uint,
        "saturating_mul",
        ReceiverMode::Value,
        SELF_ONE,
        ReturnKind::Same,
        IntrinsicOp::IntSaturatingMul,
    ),
    method(
        MethodOwner::Uint,
        "count_ones",
        ReceiverMode::Value,
        NONE,
        ReturnKind::U32,
        IntrinsicOp::IntCountOnes,
    ),
    method(
        MethodOwner::Uint,
        "leading_zeros",
        ReceiverMode::Value,
        NONE,
        ReturnKind::U32,
        IntrinsicOp::IntLeadingZeros,
    ),
    method(
        MethodOwner::Uint,
        "trailing_zeros",
        ReceiverMode::Value,
        NONE,
        ReturnKind::U32,
        IntrinsicOp::IntTrailingZeros,
    ),
    method(
        MethodOwner::Uint,
        "rotate_left",
        ReceiverMode::Value,
        U32_ONE,
        ReturnKind::Same,
        IntrinsicOp::IntRotateLeft,
    ),
    method(
        MethodOwner::Uint,
        "rotate_right",
        ReceiverMode::Value,
        U32_ONE,
        ReturnKind::Same,
        IntrinsicOp::IntRotateRight,
    ),
    method(
        MethodOwner::Uint,
        "swap_bytes",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Same,
        IntrinsicOp::IntSwapBytes,
    ),
    method(
        MethodOwner::Float,
        "abs",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Same,
        IntrinsicOp::FloatAbs,
    ),
    method(
        MethodOwner::Float,
        "min",
        ReceiverMode::Value,
        SELF_ONE,
        ReturnKind::Same,
        IntrinsicOp::FloatMin,
    ),
    method(
        MethodOwner::Float,
        "max",
        ReceiverMode::Value,
        SELF_ONE,
        ReturnKind::Same,
        IntrinsicOp::FloatMax,
    ),
    method(
        MethodOwner::Float,
        "clamp",
        ReceiverMode::Value,
        SELF_TWO,
        ReturnKind::Same,
        IntrinsicOp::FloatClamp,
    ),
    method(
        MethodOwner::Float,
        "is_nan",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Bool,
        IntrinsicOp::FloatIsNan,
    ),
    method(
        MethodOwner::Float,
        "is_finite",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Bool,
        IntrinsicOp::FloatIsFinite,
    ),
    method(
        MethodOwner::Float,
        "is_infinite",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Bool,
        IntrinsicOp::FloatIsInfinite,
    ),
    method(
        MethodOwner::Float,
        "sqrt",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Same,
        IntrinsicOp::FloatSqrt,
    ),
    method(
        MethodOwner::Float,
        "floor",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Same,
        IntrinsicOp::FloatFloor,
    ),
    method(
        MethodOwner::Float,
        "ceil",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Same,
        IntrinsicOp::FloatCeil,
    ),
    method(
        MethodOwner::Float,
        "round",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Same,
        IntrinsicOp::FloatRound,
    ),
    method(
        MethodOwner::Float,
        "trunc",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Same,
        IntrinsicOp::FloatTrunc,
    ),
    method(
        MethodOwner::Float,
        "fract",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Same,
        IntrinsicOp::FloatFract,
    ),
    method(
        MethodOwner::Float,
        "powi",
        ReceiverMode::Value,
        &[ParamKind::I32],
        ReturnKind::Same,
        IntrinsicOp::FloatPowi,
    ),
    method(
        MethodOwner::Bool,
        "not",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Bool,
        IntrinsicOp::BoolNot,
    ),
    method(
        MethodOwner::Bool,
        "to_i32",
        ReceiverMode::Value,
        NONE,
        ReturnKind::I32,
        IntrinsicOp::BoolToI32,
    ),
    method(
        MethodOwner::Bool,
        "toggle",
        ReceiverMode::Exclusive,
        NONE,
        ReturnKind::Unit,
        IntrinsicOp::BoolToggle,
    ),
    method(
        MethodOwner::Char,
        "is_ascii",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Bool,
        IntrinsicOp::CharIsAscii,
    ),
    method(
        MethodOwner::Char,
        "is_ascii_alphabetic",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Bool,
        IntrinsicOp::CharIsAsciiAlphabetic,
    ),
    method(
        MethodOwner::Char,
        "is_ascii_digit",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Bool,
        IntrinsicOp::CharIsAsciiDigit,
    ),
    method(
        MethodOwner::Char,
        "is_ascii_whitespace",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Bool,
        IntrinsicOp::CharIsAsciiWhitespace,
    ),
    method(
        MethodOwner::Char,
        "to_ascii_uppercase",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Same,
        IntrinsicOp::CharToAsciiUppercase,
    ),
    method(
        MethodOwner::Char,
        "to_ascii_lowercase",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Same,
        IntrinsicOp::CharToAsciiLowercase,
    ),
    method(
        MethodOwner::Char,
        "len_utf8",
        ReceiverMode::Value,
        NONE,
        ReturnKind::Usize,
        IntrinsicOp::CharLenUtf8,
    ),
    method(
        MethodOwner::Char,
        "to_u32",
        ReceiverMode::Value,
        NONE,
        ReturnKind::U32,
        IntrinsicOp::CharToU32,
    ),
    method(
        MethodOwner::Array,
        "len",
        ReceiverMode::Shared,
        NONE,
        ReturnKind::Usize,
        IntrinsicOp::Len,
    ),
    method(
        MethodOwner::Array,
        "is_empty",
        ReceiverMode::Shared,
        NONE,
        ReturnKind::Bool,
        IntrinsicOp::IsEmpty,
    ),
    method(
        MethodOwner::Array,
        "swap",
        ReceiverMode::Exclusive,
        USIZE_TWO,
        ReturnKind::Unit,
        IntrinsicOp::Swap,
    ),
    method(
        MethodOwner::Array,
        "reverse",
        ReceiverMode::Exclusive,
        NONE,
        ReturnKind::Unit,
        IntrinsicOp::Reverse,
    ),
    fill(MethodOwner::Array),
    method(
        MethodOwner::Slice,
        "len",
        ReceiverMode::Shared,
        NONE,
        ReturnKind::Usize,
        IntrinsicOp::Len,
    ),
    method(
        MethodOwner::Slice,
        "is_empty",
        ReceiverMode::Shared,
        NONE,
        ReturnKind::Bool,
        IntrinsicOp::IsEmpty,
    ),
    method(
        MethodOwner::Slice,
        "swap",
        ReceiverMode::Exclusive,
        USIZE_TWO,
        ReturnKind::Unit,
        IntrinsicOp::Swap,
    ),
    method(
        MethodOwner::Slice,
        "reverse",
        ReceiverMode::Exclusive,
        NONE,
        ReturnKind::Unit,
        IntrinsicOp::Reverse,
    ),
    fill(MethodOwner::Slice),
];

/// Looks up a method by owner and name.
pub fn find(owner: MethodOwner, name: &str) -> Option<MethodId> {
    METHODS
        .iter()
        .position(|method| method.owner == owner && method.name == name)
        .map(MethodId::from_index)
}

/// Returns every method belonging to `owner`, in catalog order.
pub fn methods_of(owner: MethodOwner) -> impl Iterator<Item = (MethodId, &'static MethodDef)> {
    METHODS
        .iter()
        .enumerate()
        .filter(move |(_, method)| method.owner == owner)
        .map(|(index, method)| (MethodId::from_index(index), method))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{METHODS, MethodOwner, find, methods_of};

    #[test]
    fn names_are_unique_within_each_owner() {
        for owner in [
            MethodOwner::Int,
            MethodOwner::Uint,
            MethodOwner::Float,
            MethodOwner::Bool,
            MethodOwner::Char,
            MethodOwner::Array,
            MethodOwner::Slice,
        ] {
            let mut names = BTreeSet::new();
            for (_, method) in methods_of(owner) {
                assert!(
                    names.insert(method.name),
                    "duplicate {owner:?}.{}",
                    method.name
                );
            }
        }
    }

    #[test]
    fn lookup_returns_the_catalog_entry() {
        let id = find(MethodOwner::Slice, "reverse").expect("slice reverse exists");
        assert_eq!(id.def().name, "reverse");
        assert_eq!(id.def().intrinsic, super::IntrinsicOp::Reverse);
    }

    #[test]
    fn fill_requires_copy_elements_on_sequences() {
        for owner in [MethodOwner::Array, MethodOwner::Slice] {
            let id = find(owner, "fill").expect("fill exists");
            assert!(id.def().requires_copy_element);
        }
    }

    #[test]
    fn every_catalog_entry_is_reachable_by_owner_and_name() {
        for (index, method) in METHODS.iter().enumerate() {
            assert_eq!(
                find(method.owner, method.name).map(|id| id.index()),
                Some(index)
            );
        }
    }
}
