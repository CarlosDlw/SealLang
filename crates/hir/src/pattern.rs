//! Lowered patterns and match arms.
//!
//! By the time a pattern reaches this representation, every enum, variant, and
//! field is resolved and every binding has a definition. Later phases only walk
//! the tree; they never look a name up again.

use seal_ast::PatternLiteral;
use seal_def::DefId;
use seal_span::Span;
use seal_symbol::Symbol;
use seal_ty::VariantIdx;

use crate::id::{HirExprId, HirPatternId};

/// A lowered pattern.
#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    /// What the pattern matches.
    pub kind: PatternKind,
    /// The source range of the whole pattern.
    pub span: Span,
}

/// The kinds of lowered pattern.
#[derive(Clone, Debug, PartialEq)]
pub enum PatternKind {
    /// `_`, which matches anything and binds nothing.
    Wildcard,
    /// A name that binds the matched value.
    Binding {
        /// The bound name.
        name: Symbol,
        /// The binding's definition, which carries its scope and mutability.
        def: DefId,
    },
    /// A scalar literal compared by value.
    Literal(PatternLiteral),
    /// An enum variant with its fields in declaration order.
    ///
    /// A named pattern may be written in any order; lowering reorders it here so
    /// every consumer sees one canonical shape.
    Variant {
        /// The enum definition.
        adt: DefId,
        /// The variant's position in declaration order.
        variant: VariantIdx,
        /// One entry per declared field: the field definition and its pattern.
        fields: Vec<(DefId, HirPatternId)>,
    },
    /// Alternatives that bind the same names.
    Or(Vec<HirPatternId>),
    /// A pattern that could not be resolved, after a diagnostic was reported.
    Error,
}

/// One lowered arm of a `match` expression.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchArm {
    /// The pattern that selects this arm.
    pub pattern: HirPatternId,
    /// The optional guard, evaluated after the pattern matches.
    pub guard: Option<HirExprId>,
    /// The arm body.
    pub body: HirExprId,
    /// The source range of the whole arm.
    pub span: Span,
}
