//! Values computed by statements.

use seal_def::DefId;

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

/// The aggregate being built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AggregateKind {
    /// A struct, identified by its definition.
    Adt(DefId),
}

/// An expression that produces a value and can be assigned to a place.
#[derive(Clone, Debug, PartialEq)]
pub enum Rvalue {
    /// Copies or moves an operand.
    Use(Operand),
    /// Borrows a place, written `&place`.
    ///
    /// The resulting `*T` is a shared reference: it keeps the borrowed place
    /// alive for as long as the reference is used, and it permits other shared
    /// reads of the same place.
    Ref(Place),
    /// Applies a binary operator to two operands.
    Binary {
        /// The operator.
        op: BinOp,
        /// The left operand.
        lhs: Operand,
        /// The right operand.
        rhs: Operand,
    },
    /// Applies a unary operator to an operand.
    Unary {
        /// The operator.
        op: UnOp,
        /// The operand.
        operand: Operand,
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
}
