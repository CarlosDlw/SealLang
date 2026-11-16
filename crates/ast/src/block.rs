//! Blocks, statements, and `let` bindings.

use seal_span::Span;
use seal_symbol::Symbol;

use crate::id::{BlockId, ExprId, StmtId, TypeId};

/// A braced block.
///
/// `tail` is the optional final expression without a semicolon; it is the
/// block's value. Statements before it run for their effects.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    /// Statements in source order.
    pub statements: Vec<StmtId>,
    /// The trailing expression, if the block has one.
    pub tail: Option<ExprId>,
    /// The block's source range, braces included.
    pub span: Span,
}

/// A statement.
#[derive(Clone, Debug, PartialEq)]
pub struct Stmt {
    /// What the statement is.
    pub kind: StmtKind,
    /// The statement's source range.
    pub span: Span,
}

/// The kinds of statement.
#[derive(Clone, Debug, PartialEq)]
pub enum StmtKind {
    /// A `let` binding.
    Let(LetStmt),
    /// A `return`, with an optional value.
    Return(Option<ExprId>),
    /// A `while` loop.
    While {
        /// Evaluated before every iteration; it must have type `bool`.
        condition: ExprId,
        /// The loop body.
        body: BlockId,
    },
    /// A `break`, which exits the innermost loop.
    Break,
    /// A `continue`, which starts the next iteration of the innermost loop.
    Continue,
    /// An expression evaluated for its effects; it ended with `;`, or it is a
    /// block-like expression used as a statement.
    Expr(ExprId),
    /// Syntax the parser could not turn into a statement.
    Error,
}

/// A `let` binding.
#[derive(Clone, Debug, PartialEq)]
pub struct LetStmt {
    /// The bound name.
    pub name: Symbol,
    /// The span of the name.
    pub name_span: Span,
    /// Whether the binding was declared `mut` and can be reassigned.
    pub mutable: bool,
    /// The optional type annotation.
    pub ty: Option<TypeId>,
    /// The optional initializer.
    pub init: Option<ExprId>,
}
