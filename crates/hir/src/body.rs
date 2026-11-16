//! Lowered bodies, blocks, and statements.

use seal_def::DefId;
use seal_span::Span;
use seal_ty::TyId;

use crate::id::{HirBlockId, HirExprId, HirStmtId};

/// An executable body, owned by a function.
#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    /// The function this body implements.
    pub owner: DefId,
    /// The declared return type; `None` means unit.
    pub ret: Option<TyId>,
    /// Parameters in source order.
    pub params: Vec<ParamData>,
    /// The root block.
    pub root: HirBlockId,
}

/// A lowered parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParamData {
    /// The parameter's definition, which carries its name and spans.
    pub def: DefId,
    /// The lowered declared type.
    pub ty: TyId,
}

/// A lowered block.
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    /// Statements in source order.
    pub statements: Vec<HirStmtId>,
    /// The block's value, if it has a tail expression.
    pub tail: Option<HirExprId>,
    /// The block's source range, braces included.
    pub span: Span,
}

/// A lowered statement.
#[derive(Clone, Debug, PartialEq)]
pub struct Stmt {
    /// What the statement is.
    pub kind: StmtKind,
    /// The statement's source range.
    pub span: Span,
}

/// The kinds of lowered statement.
#[derive(Clone, Debug, PartialEq)]
pub enum StmtKind {
    /// A `let` binding.
    ///
    /// The binding is not in scope in its own initializer, so `let x = x;`
    /// reads an outer `x`, exactly as written.
    Let {
        /// The binding's definition.
        local: DefId,
        /// The lowered type annotation, if any.
        ty: Option<TyId>,
        /// The initializer, if any.
        init: Option<HirExprId>,
    },
    /// A `return`, with an optional value.
    Return(Option<HirExprId>),
    /// A `while` loop.
    While {
        /// Evaluated before every iteration; it must have type `bool`.
        condition: HirExprId,
        /// The loop body.
        body: HirBlockId,
    },
    /// A `break`, which exits the innermost loop.
    Break,
    /// A `continue`, which starts the next iteration of the innermost loop.
    Continue,
    /// An expression evaluated for its effects.
    Expr(HirExprId),
    /// Syntax that could not be lowered, after a diagnostic was reported.
    Error,
}
