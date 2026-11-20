//! Typed `cfg(...)` conditions.
//!
//! The parser validates the shape and known keys; the project/build layer later
//! evaluates a condition for one target and feature set. Keeping the expression
//! typed prevents the silent `false` produced by a misspelled string in a
//! string-based configuration language.

use seal_span::Span;
use seal_symbol::Symbol;

use crate::id::CfgExprId;

/// One parsed `cfg(...)` expression.
#[derive(Clone, Debug, PartialEq)]
pub struct CfgExpr {
    /// What the expression tests.
    pub kind: CfgExprKind,
    /// The expression's exact source range.
    pub span: Span,
}

/// The kinds of `cfg(...)` expression.
#[derive(Clone, Debug, PartialEq)]
pub enum CfgExprKind {
    /// `feature("name")`.
    Feature(Symbol),
    /// `env("NAME")`.
    Env(Symbol),
    /// `target.os`, `target.arch`, `target.env`, or `target.pointer_width`.
    Target(Symbol),
    /// `debug_assertions`.
    DebugAssertions,
    /// `test`.
    Test,
    /// `bench`.
    Bench,
    /// A literal atom used on the right side of a comparison.
    Literal(CfgLiteral),
    /// `lhs == rhs` or `lhs != rhs`.
    Compare {
        /// The left operand.
        lhs: CfgExprId,
        /// The comparison operator.
        op: CfgCmpOp,
        /// The right operand.
        rhs: CfgExprId,
    },
    /// `lhs && rhs`.
    And {
        /// The left operand.
        lhs: CfgExprId,
        /// The right operand.
        rhs: CfgExprId,
    },
    /// `lhs || rhs`.
    Or {
        /// The left operand.
        lhs: CfgExprId,
        /// The right operand.
        rhs: CfgExprId,
    },
    /// `!operand`.
    Not(CfgExprId),
    /// Syntax the parser could not turn into a condition.
    Error,
}

/// A literal value written inside a `cfg(...)` comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CfgLiteral {
    /// A string literal.
    Str(Symbol),
    /// A boolean literal.
    Bool(bool),
    /// An integer literal.
    Int(u128),
}

/// The comparison operators accepted by `cfg(...)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CfgCmpOp {
    /// `==`
    Eq,
    /// `!=`
    Ne,
}

impl CfgCmpOp {
    /// Returns the source spelling of the operator.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Eq => "==",
            Self::Ne => "!=",
        }
    }
}
