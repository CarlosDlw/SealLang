//! Expressions, operators, and literals.

use seal_span::Span;
use seal_symbol::Symbol;

use crate::id::{BlockId, ExprId};

/// A binary operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinaryOp {
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
    /// `&&`
    And,
    /// `||`
    Or,
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

impl BinaryOp {
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
            Self::And => "&&",
            Self::Or => "||",
            Self::BitAnd => "&",
            Self::BitOr => "|",
            Self::BitXor => "^",
            Self::Shl => "<<",
            Self::Shr => ">>",
        }
    }
}

/// A prefix operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    /// `-`
    Neg,
    /// `!`
    Not,
    /// `~`
    BitNot,
}

impl UnaryOp {
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

/// An assignment operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssignOp {
    /// `=`
    Assign,
    /// `+=`
    AddAssign,
    /// `-=`
    SubAssign,
    /// `*=`
    MulAssign,
    /// `/=`
    DivAssign,
    /// `%=`
    RemAssign,
    /// `<<=`
    ShlAssign,
    /// `>>=`
    ShrAssign,
    /// `&=`
    BitAndAssign,
    /// `|=`
    BitOrAssign,
    /// `^=`
    BitXorAssign,
}

impl AssignOp {
    /// Returns the source spelling of the operator.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Assign => "=",
            Self::AddAssign => "+=",
            Self::SubAssign => "-=",
            Self::MulAssign => "*=",
            Self::DivAssign => "/=",
            Self::RemAssign => "%=",
            Self::ShlAssign => "<<=",
            Self::ShrAssign => ">>=",
            Self::BitAndAssign => "&=",
            Self::BitOrAssign => "|=",
            Self::BitXorAssign => "^=",
        }
    }
}

/// An increment or decrement operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UpdateOp {
    /// `++`
    Increment,
    /// `--`
    Decrement,
}

impl UpdateOp {
    /// Returns the source spelling of the operator.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Increment => "++",
            Self::Decrement => "--",
        }
    }
}

/// A literal value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Literal {
    /// An integer literal, with its radix and optional type suffix.
    Int {
        /// Value of the digits, separators ignored.
        value: u128,
        /// Radix written in the source: 2, 8, 10, or 16.
        base: u32,
        /// Type suffix, if any.
        suffix: Option<Symbol>,
    },
    /// A floating-point literal and its optional type suffix.
    Float {
        /// The correctly rounded value.
        value: f64,
        /// Type suffix, if any.
        suffix: Option<Symbol>,
    },
    /// A string literal; the symbol holds the unescaped contents.
    Str(Symbol),
    /// A character literal.
    Char(char),
    /// A boolean literal.
    Bool(bool),
}

/// An expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    /// What the expression is.
    pub kind: ExprKind,
    /// The expression's source range.
    pub span: Span,
}

/// The kinds of expression.
#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    /// A literal.
    Lit(Literal),
    /// A variable reference.
    Path(Symbol),
    /// A borrow of a place, written `&place`.
    Borrow {
        /// The place being borrowed.
        operand: ExprId,
    },
    /// A read through a reference, written `*reference`.
    Deref {
        /// The reference being read.
        operand: ExprId,
    },
    /// A prefix operation.
    Unary {
        /// The operator.
        op: UnaryOp,
        /// The operand.
        operand: ExprId,
    },
    /// A binary operation.
    Binary {
        /// The operator.
        op: BinaryOp,
        /// The left operand.
        lhs: ExprId,
        /// The right operand.
        rhs: ExprId,
    },
    /// An assignment, including compound assignment.
    Assign {
        /// The operator.
        op: AssignOp,
        /// The place being assigned.
        target: ExprId,
        /// The new value.
        value: ExprId,
    },
    /// An increment or decrement.
    Update {
        /// The operator.
        op: UpdateOp,
        /// The place being updated.
        operand: ExprId,
        /// `true` for `x++`, `false` for `++x`.
        postfix: bool,
    },
    /// A call.
    Call {
        /// The callee.
        callee: ExprId,
        /// Arguments in source order.
        args: Vec<ExprId>,
    },
    /// A parenthesized expression. Kept so formatting and spans stay faithful.
    Paren(ExprId),
    /// A conditional expression.
    ///
    /// The value of the expression is the value of the branch that runs. An
    /// `else if` chain is represented as a block whose tail is another `If`, so
    /// every branch is a block and later phases see one shape.
    If {
        /// The condition, which must have type `bool`.
        condition: ExprId,
        /// The block evaluated when the condition is `true`.
        then_branch: BlockId,
        /// The block evaluated when the condition is `false`, if any.
        else_branch: Option<BlockId>,
    },
    /// Syntax the parser could not turn into an expression.
    Error,
}
