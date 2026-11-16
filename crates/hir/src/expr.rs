//! Lowered expressions and places.

use seal_ast::{AssignOp, BinaryOp, Literal, UnaryOp, UpdateOp};
use seal_def::DefId;
use seal_span::Span;

use crate::id::{HirBlockId, HirExprId};
use crate::res::Res;

/// A storage location: a local definition plus its projections.
///
/// References add dereference projections, so `*p` and `**p` are places rooted
/// at the local that holds the reference. Aggregate fields are added inside
/// [`Place`] when aggregates arrive, without changing how callers obtain one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    local: DefId,
    derefs: u32,
}

impl Place {
    /// Creates the place for a whole local.
    #[inline]
    pub const fn local(local: DefId) -> Self {
        Self { local, derefs: 0 }
    }

    /// Returns the local definition this place starts from.
    #[inline]
    pub const fn local_def(self) -> DefId {
        self.local
    }

    /// Returns the place reached by one dereference.
    ///
    /// # Panics
    ///
    /// Panics if the projection count would overflow `u32`. The parser bounds
    /// nesting far below this, so the panic is an internal invariant check.
    #[inline]
    pub fn deref(self) -> Self {
        Self {
            local: self.local,
            derefs: self.derefs + 1,
        }
    }

    /// Returns the number of dereference projections applied to the local.
    #[inline]
    pub const fn derefs(self) -> u32 {
        self.derefs
    }
}

/// A lowered expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    /// What the expression is.
    pub kind: ExprKind,
    /// The expression's source range.
    pub span: Span,
}

/// The kinds of lowered expression.
#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    /// A literal.
    Lit(Literal),
    /// A resolved name used as a value.
    Path(Res),
    /// A borrow of a place, written `&place`.
    Borrow {
        /// The place being borrowed.
        operand: HirExprId,
    },
    /// A read through a reference, written `*reference`.
    Deref {
        /// The reference being read.
        operand: HirExprId,
    },
    /// A prefix operation.
    Unary {
        /// The operator.
        op: UnaryOp,
        /// The operand.
        operand: HirExprId,
    },
    /// A binary operation.
    Binary {
        /// The operator.
        op: BinaryOp,
        /// The left operand.
        lhs: HirExprId,
        /// The right operand.
        rhs: HirExprId,
    },
    /// An assignment, including compound assignment.
    ///
    /// `target` is kept as an expression because it may fail to resolve; use
    /// [`Hir::place`](crate::Hir::place) to ask for the place it denotes.
    Assign {
        /// The operator.
        op: AssignOp,
        /// The place being assigned.
        target: HirExprId,
        /// The new value.
        value: HirExprId,
    },
    /// An increment or decrement.
    Update {
        /// The operator.
        op: UpdateOp,
        /// The place being updated.
        target: HirExprId,
        /// `true` for `x++`, `false` for `++x`.
        postfix: bool,
    },
    /// A call.
    Call {
        /// The callee.
        callee: HirExprId,
        /// Arguments in source order.
        args: Vec<HirExprId>,
    },
    /// A conditional expression.
    If {
        /// The condition, which must have type `bool`.
        condition: HirExprId,
        /// The block evaluated when the condition is `true`.
        then_branch: HirBlockId,
        /// The block evaluated when the condition is `false`, if any.
        else_branch: Option<HirBlockId>,
    },
    /// Syntax that could not be lowered, after a diagnostic was reported.
    Error,
}
