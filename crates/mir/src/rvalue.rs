//! Values computed by statements.

use seal_def::DefId;
use seal_methods::MethodId;
use seal_span::Span;
use seal_ty::VariantIdx;

use crate::operand::Operand;
use crate::place::Place;

/// A binary operator in MIR.
///
/// Short-circuiting `&&` and `||` are not binary operations: they lower to
/// `SwitchInt`, so the right operand is evaluated only when required.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Rem,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    Le,
    /// `>=`
    Ge,
    /// `&`
    BitAnd,
    /// `|`
    BitOr,
    /// `^`
    BitXor,
    /// `<<`
    Shl,
    /// `>>`
    Shr,
}

impl BinOp {
    /// Returns the source spelling of the operator.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Rem => "%",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Gt => ">",
            Self::Le => "<=",
            Self::Ge => ">=",
            Self::BitAnd => "&",
            Self::BitOr => "|",
            Self::BitXor => "^",
            Self::Shl => "<<",
            Self::Shr => ">>",
        }
    }
}

/// A unary operator in MIR.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnOp {
    /// Arithmetic negation.
    Neg,
    /// Logical negation.
    Not,
    /// Bitwise complement.
    BitNot,
}

impl UnOp {
    /// Returns the source spelling of the operator.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Neg => "-",
            Self::Not => "!",
            Self::BitNot => "~",
        }
    }
}

/// How an intrinsic method receives its subject.
#[derive(Clone, Debug, PartialEq)]
pub enum IntrinsicReceiver {
    /// A place holding a scalar, array, or aggregate. The method definition
    /// decides whether the call reads it or writes it.
    Place(Place),
    /// A slice fat pointer. The pointee is unsized, so the value itself is the
    /// receiver and the data pointer and length are extracted at codegen.
    Value(Operand),
}

/// The aggregate being built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AggregateKind {
    /// A struct, identified by its definition.
    Struct(DefId),
    /// One variant of an enum, identified by the enum definition and the
    /// variant's position in declaration order.
    Variant {
        /// The enum definition.
        adt: DefId,
        /// The variant's discriminant.
        variant: VariantIdx,
    },
}

/// An expression that produces a value and can be assigned to a place.
#[derive(Clone, Debug, PartialEq)]
pub enum Rvalue {
    /// Copies or moves an operand.
    Use(Operand),
    /// Borrows a place, written `&place` or `&mut place`.
    ///
    /// A shared reference (`*T`) keeps the borrowed place alive and permits
    /// other shared reads of it; an exclusive one (`*mut T`) permits reads and
    /// writes but conflicts with every other loan. Borrow checking compares
    /// loans by kind.
    Ref {
        /// The borrowed place.
        place: Place,
        /// Whether the borrow is exclusive.
        kind: seal_ty::RefKind,
    },
    /// Builds a slice reference from an array or slice place.
    ///
    /// `start` and `end` are `usize` values; an absent bound means the start of
    /// the source or its full length. Lowering checks `start <= end <= length`
    /// before this value is constructed.
    SliceRef {
        /// The array or slice the range selects from.
        base: Place,
        /// The inclusive start bound.
        start: Option<Operand>,
        /// The exclusive end bound.
        end: Option<Operand>,
        /// Whether the reference permits writes.
        kind: seal_ty::RefKind,
        /// The resulting `*[T]` or `*mut [T]` type.
        ty: seal_ty::TyId,
    },
    /// Reads the discriminant of an enum place.
    ///
    /// The result is a `u32`, wide enough for every supported discriminant, so
    /// a `SwitchInt` can compare it directly. Reading the discriminant never
    /// moves the enum.
    Discriminant(Place),
    /// Reads the length of a slice reference.
    Len {
        /// The fat pointer whose length is read.
        slice: Operand,
    },
    /// Applies a binary operator to two operands.
    Binary {
        /// The operator.
        op: BinOp,
        /// The left operand.
        lhs: Operand,
        /// The right operand.
        rhs: Operand,
        /// The source range of the whole binary expression.
        span: Span,
    },
    /// Applies a unary operator to an operand.
    Unary {
        /// The operator.
        op: UnOp,
        /// The operand.
        operand: Operand,
    },
    /// Calls a compiler-owned intrinsic method.
    ///
    /// The receiver is part of the call, not a preceding borrow: a shared
    /// method reads it and an exclusive method writes it for the duration of
    /// the statement. No reference escapes, so the borrow checker checks the
    /// access directly instead of creating a loan.
    Intrinsic {
        /// The resolved catalog entry.
        method: MethodId,
        /// The receiver, by place or as a slice fat pointer.
        receiver: IntrinsicReceiver,
        /// Arguments in evaluation order, excluding the receiver.
        args: Vec<Operand>,
        /// The source range of the whole method call.
        span: Span,
    },
    /// Calls a function.
    ///
    /// Calls are rvalues because Seal has no unwinding and a call cannot fail in
    /// a way that changes control flow. A future ABI with unwind edges will turn
    /// this into a terminator.
    Call {
        /// The called function.
        func: DefId,
        /// Arguments in evaluation order.
        args: Vec<Operand>,
        /// The source range of each argument, matching `args` by position.
        ///
        /// Call diagnostics point at the argument that violates the contract
        /// instead of the whole call, so lowering records one span per operand
        /// and validation keeps the two lists in step.
        arg_spans: Vec<Span>,
    },
    /// Builds an aggregate value from its fields.
    ///
    /// Field operands appear in source order, which is evaluation order. A
    /// field that is not `Copy` is moved out of its place, so building an
    /// aggregate can end the lifetime of the places it reads.
    Aggregate {
        /// What is being built.
        kind: AggregateKind,
        /// Field initializers in evaluation order.
        fields: Vec<(DefId, Operand)>,
    },
    /// Builds a fixed-size array from its elements.
    Array {
        /// Elements in source order.
        elements: Vec<Operand>,
        /// The array type, which carries the element type and length.
        ty: seal_ty::TyId,
    },
    /// Builds an array by copying one value into every element.
    Repeat {
        /// The value copied into each element.
        value: Operand,
        /// The number of elements.
        count: u64,
        /// The array type, which carries the element type and length.
        ty: seal_ty::TyId,
    },
}
