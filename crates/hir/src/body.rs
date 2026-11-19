//! Lowered bodies, blocks, and statements.

use seal_def::DefId;
use seal_span::Span;
use seal_ty::TyId;

use crate::id::{HirBlockId, HirExprId, HirPatternId, HirStmtId};

/// What an executable body belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    /// A function body. It may have parameters.
    Function,
    /// A constant initializer. It has no parameters.
    Const,
    /// A static initializer. It has no parameters.
    Static {
        /// Whether the static is declared `mut`.
        mutable: bool,
    },
}

/// An executable body, owned by a function, constant, or static.
#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    /// The definition this body implements.
    pub owner: DefId,
    /// What kind of initializer or function the owner declares.
    pub kind: BodyKind,
    /// The declared return type; `None` means unit.
    pub ret: Option<TyId>,
    /// Whether this is the body of a `const func`.
    ///
    /// Only a [`BodyKind::Function`] may be const; value initializers and
    /// trait default bodies are always `false`.
    pub is_const: bool,
    /// Whether this is the body of an `unsafe func`.
    ///
    /// The flag belongs to the signature. It does not make the body an unsafe
    /// context: every unsafe operation inside still needs an `unsafe` block.
    pub is_unsafe: bool,
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

/// What a lowered `for` loop iterates.
#[derive(Clone, Debug, PartialEq)]
pub enum ForSource {
    /// An integer range. Both bounds have the same integer type.
    Range {
        /// The first value, included.
        start: HirExprId,
        /// The end value, excluded or included according to `inclusive`.
        end: HirExprId,
        /// `true` for `..=`, `false` for `..`.
        inclusive: bool,
    },
    /// An array, or a reference to an array or slice.
    Expr(HirExprId),
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
    /// A block-scope constant declaration.
    ///
    /// The initializer is the body owned by `def`, not an expression of the
    /// enclosing function.
    Const {
        /// The constant's definition.
        def: DefId,
    },
    /// A block-scope static declaration.
    ///
    /// The initializer is the body owned by `def`, shared by every execution of
    /// the declaration.
    Static {
        /// The static's definition.
        def: DefId,
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
    /// A `while let` loop.
    WhileLet {
        /// Matched against the scrutinee before every iteration.
        pattern: HirPatternId,
        /// Evaluated before every iteration.
        scrutinee: HirExprId,
        /// The loop body.
        body: HirBlockId,
    },
    /// A `for` loop over an iterable value.
    For {
        /// Matched against each element; type checking requires it to be
        /// irrefutable.
        pattern: crate::id::HirPatternId,
        /// What the loop iterates.
        source: ForSource,
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
