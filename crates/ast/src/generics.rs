//! Generic declarations, bounds, type arguments, and where clauses.
//!
//! These nodes are syntax only. Name resolution and the semantic identities of
//! parameters belong to HIR; this module records what the source wrote and where
//! every delimiter and name was written.

use seal_span::Span;
use seal_symbol::Symbol;

use crate::id::{BoundId, GenericParamId, TypeId};

/// The `<...>` parameter list written after an item or method name.
#[derive(Clone, Debug, PartialEq)]
pub struct GenericParams {
    /// Parameters in declaration order.
    pub params: Vec<GenericParamId>,
    /// The span from `<` through the matching `>`, or an empty span when the
    /// list is absent.
    pub span: Span,
}

impl GenericParams {
    /// Creates an absent parameter list anchored at `span`.
    pub fn empty(span: Span) -> Self {
        Self {
            params: Vec::new(),
            span,
        }
    }

    /// Returns the number of declared parameters.
    pub fn len(&self) -> usize {
        self.params.len()
    }

    /// Returns `true` when no parameter list was written.
    pub fn is_empty(&self) -> bool {
        self.params.is_empty()
    }
}

/// One declared type parameter and the bounds attached to it inline.
#[derive(Clone, Debug, PartialEq)]
pub struct GenericParamDecl {
    /// The parameter name.
    pub name: Symbol,
    /// The span of the name.
    pub name_span: Span,
    /// Inline bounds in source order.
    pub bounds: Vec<BoundId>,
    /// The span from the name through the last bound.
    pub span: Span,
}

/// The `<...>` type arguments written after a type or trait name.
#[derive(Clone, Debug, PartialEq)]
pub struct GenericArgs {
    /// Arguments in source order.
    pub args: Vec<TypeId>,
    /// The span from `<` through the matching `>`.
    pub span: Span,
}

/// A named type path with optional generic arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct TypePath {
    /// The name as written.
    pub name: Symbol,
    /// The span of the name.
    pub name_span: Span,
    /// Explicit type arguments, or `None` when the source omitted them.
    pub args: Option<GenericArgs>,
}

impl TypePath {
    /// Creates a path without arguments.
    pub fn new(name: Symbol, name_span: Span) -> Self {
        Self {
            name,
            name_span,
            args: None,
        }
    }
}

/// One segment of a multi-segment type or value path.
#[derive(Clone, Debug, PartialEq)]
pub struct PathSegment {
    /// The segment name.
    pub name: Symbol,
    /// The span of the name.
    pub name_span: Span,
    /// Explicit arguments written on this segment, if any.
    pub args: Option<GenericArgs>,
}

/// A trait name with optional generic arguments.
#[derive(Clone, Debug, PartialEq)]
pub struct TraitPath {
    /// The trait name.
    pub name: Symbol,
    /// The span of the name.
    pub name_span: Span,
    /// Explicit trait arguments, or `None` when omitted.
    pub args: Option<GenericArgs>,
    /// The span from the name through the last argument.
    pub span: Span,
}

impl TraitPath {
    /// Creates a trait path without arguments.
    pub fn new(name: Symbol, name_span: Span) -> Self {
        Self {
            name,
            name_span,
            args: None,
            span: name_span,
        }
    }
}

/// A `where` clause attached to an item header.
#[derive(Clone, Debug, PartialEq)]
pub struct WhereClause {
    /// Predicates in source order.
    pub predicates: Vec<WherePredicate>,
    /// The span from `where` through the last predicate, or an empty span when
    /// the clause is absent.
    pub span: Span,
}

impl WhereClause {
    /// Creates an absent clause anchored at `span`.
    pub fn empty(span: Span) -> Self {
        Self {
            predicates: Vec::new(),
            span,
        }
    }

    /// Returns the number of predicates.
    pub fn len(&self) -> usize {
        self.predicates.len()
    }

    /// Returns `true` when no `where` clause was written.
    pub fn is_empty(&self) -> bool {
        self.predicates.is_empty()
    }
}

/// One predicate in a `where` clause, such as `T: Copy + Trait<U>`.
#[derive(Clone, Debug, PartialEq)]
pub struct WherePredicate {
    /// The constrained type.
    pub ty: TypeId,
    /// Bounds that the type must satisfy.
    pub bounds: Vec<BoundId>,
    /// The span from the type through the last bound.
    pub span: Span,
}

/// A bound written on a parameter or in a `where` predicate.
#[derive(Clone, Debug, PartialEq)]
pub struct Bound {
    /// What the bound requires.
    pub kind: BoundKind,
    /// The bound's source span.
    pub span: Span,
}

/// The kind of a bound.
#[derive(Clone, Debug, PartialEq)]
pub enum BoundKind {
    /// The built-in `Copy` bound.
    Copy,
    /// A user-declared trait with optional arguments.
    Trait(TraitPath),
}
