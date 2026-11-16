//! Lowered expressions and places.

use seal_ast::{AssignOp, BinaryOp, Literal, UnaryOp, UpdateOp};
use seal_def::DefId;
use seal_span::Span;
use seal_symbol::Symbol;

use crate::id::{HirBlockId, HirExprId};
use crate::res::Res;

/// A storage location: a local definition plus its projections.
///
/// References add dereference projections, so `*p` is the place rooted at the
/// local that holds the reference. Field names stay symbolic here: resolving
/// them needs the base type, which type checking computes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    local: DefId,
    projections: Vec<Projection>,
}

/// One step from a place to a sub-place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Projection {
    /// Reads through a reference, written `*place`.
    Deref,
    /// Selects a named field of a struct place.
    Field(Symbol),
    /// Selects an array element by an index expression.
    Index(crate::id::HirExprId),
}

impl Place {
    /// Creates the place for a whole local.
    #[inline]
    pub fn local(local: DefId) -> Self {
        Self {
            local,
            projections: Vec::new(),
        }
    }

    /// Returns the local definition this place starts from.
    #[inline]
    pub const fn local_def(&self) -> DefId {
        self.local
    }

    /// Returns the place reached by one dereference.
    #[inline]
    pub fn deref(mut self) -> Self {
        self.projections.push(Projection::Deref);
        self
    }

    /// Returns the place reached by selecting `name`.
    #[inline]
    pub fn field(mut self, name: Symbol) -> Self {
        self.projections.push(Projection::Field(name));
        self
    }

    /// Returns the place reached by indexing with `index`.
    #[inline]
    pub fn index(mut self, index: crate::id::HirExprId) -> Self {
        self.projections.push(Projection::Index(index));
        self
    }

    /// Returns the projections applied to the local, outermost last.
    #[inline]
    pub fn projections(&self) -> &[Projection] {
        &self.projections
    }

    /// Returns `true` if any projection reads through a reference.
    #[inline]
    pub fn contains_deref(&self) -> bool {
        self.projections.contains(&Projection::Deref)
    }

    /// Returns `true` if the place names the whole local.
    #[inline]
    pub fn is_whole(&self) -> bool {
        self.projections.is_empty()
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
    /// A borrow of a place, written `&place` or `&mut place`.
    Borrow {
        /// The place being borrowed.
        operand: HirExprId,
        /// Whether the borrow is exclusive (`*mut T`).
        kind: seal_ty::RefKind,
    },
    /// A read through a reference, written `*reference`.
    Deref {
        /// The reference being read.
        operand: HirExprId,
    },
    /// A field access on a place, written `base.name`.
    Field {
        /// The place the field is projected from.
        base: HirExprId,
        /// The field name, resolved against the base type by type checking.
        name: Symbol,
        /// The span of the field name.
        name_span: Span,
    },
    /// A struct literal with resolved fields.
    Struct {
        /// The struct definition named by the literal.
        adt: DefId,
        /// Initializers in source order, one per valid source field.
        fields: Vec<(DefId, HirExprId)>,
    },
    /// An array literal.
    Array {
        /// Elements in source order; an empty literal has no elements.
        elements: Vec<HirExprId>,
    },
    /// A repeated array literal, written `[value; length]`.
    ArrayRepeat {
        /// The value copied into each element.
        value: HirExprId,
        /// The number of elements.
        length: u64,
    },
    /// An array index place, written `base[index]`.
    Index {
        /// The array place being indexed.
        base: HirExprId,
        /// The index expression.
        index: HirExprId,
    },
    /// A slice range place, written `base[start..end]`.
    ///
    /// The range has unsized type `[T]` and is only useful as the operand of a
    /// borrow, which lowers to [`ExprKind::SliceRef`].
    Slice {
        /// The array or slice place being sliced.
        base: HirExprId,
        /// The inclusive start bound, or the beginning when absent.
        start: Option<HirExprId>,
        /// The exclusive end bound, or the source length when absent.
        end: Option<HirExprId>,
    },
    /// A borrow of a slice range, lowered directly to a fat reference.
    SliceRef {
        /// The array or slice place being sliced.
        base: HirExprId,
        /// The inclusive start bound, or the beginning when absent.
        start: Option<HirExprId>,
        /// The exclusive end bound, or the source length when absent.
        end: Option<HirExprId>,
        /// Whether the reference permits writes.
        kind: seal_ty::RefKind,
    },
    /// The length of a slice reference, written `len(slice)`.
    Len {
        /// The `*[T]` or `*mut [T]` value whose length is read.
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
