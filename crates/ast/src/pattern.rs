//! Patterns and match arms.
//!
//! Patterns are a separate arena because they are a tree of their own: match
//! arms, bindings, and nested enum fields all refer to [`PatternId`]s, and a
//! later phase can walk the pattern tree without re-reading expressions.

use seal_span::Span;
use seal_symbol::Symbol;

use crate::id::{ExprId, PatternId};

/// A pattern, with the span of its source text.
#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    /// What the pattern matches.
    pub kind: PatternKind,
    /// The source range of the whole pattern.
    pub span: Span,
}

/// A scalar literal allowed in a pattern.
///
/// String patterns are not part of the grammar: `str` is an unsized pointee,
/// and comparing borrowed string contents would need a runtime operation that
/// the language does not define. Integer, float, character, and boolean
/// literals are compared by value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PatternLiteral {
    /// An integer literal, with an optional sign and type suffix.
    Int {
        /// Value of the digits, separators ignored.
        value: u128,
        /// Whether the source wrote a leading `-`.
        negative: bool,
        /// Type suffix, if any.
        suffix: Option<Symbol>,
    },
    /// A floating-point literal, with an optional type suffix.
    Float {
        /// The correctly rounded value, already signed.
        value: f64,
        /// Type suffix, if any.
        suffix: Option<Symbol>,
    },
    /// A character literal.
    Char(char),
    /// A boolean literal.
    Bool(bool),
}

/// The kinds of pattern.
#[derive(Clone, Debug, PartialEq)]
pub enum PatternKind {
    /// `_`, which matches anything without binding it.
    Wildcard,
    /// A binding such as `value`, optionally constrained by `name @ pattern`.
    Binding {
        /// The bound name.
        name: Symbol,
        /// The subpattern after `@`, if any.
        pattern: Option<PatternId>,
    },
    /// A scalar literal such as `0`, `-1`, `'x'`, or `true`.
    Literal(PatternLiteral),
    /// An inclusive or exclusive scalar range such as `0..=9` or `'a'..'z'`.
    Range {
        /// The first value of the range.
        start: PatternLiteral,
        /// The exclusive or inclusive end value.
        end: PatternLiteral,
        /// `true` for `..=`, `false` for `..`.
        inclusive: bool,
    },
    /// A qualified enum variant such as `Option::Some(value)`.
    Variant {
        /// The enum name.
        type_name: Symbol,
        /// The span of the enum name.
        type_span: Span,
        /// The variant name.
        name: Symbol,
        /// The span of the variant name.
        name_span: Span,
        /// The fields matched by the pattern.
        fields: VariantPatternFields,
    },
    /// A struct destructuring pattern such as `Point { x, y: 0 }` or
    /// `Point { x, .. }`.
    Struct {
        /// The struct name.
        name: Symbol,
        /// The span of the struct name.
        name_span: Span,
        /// The fields matched by the pattern, in source order.
        fields: Vec<NamedPatternField>,
        /// Whether the pattern ends with `..`, ignoring the other fields.
        has_rest: bool,
    },
    /// A fixed-size array pattern such as `[first, second]` or `[first, .., last]`.
    Array {
        /// Patterns before the `..`, if any.
        prefix: Vec<PatternId>,
        /// Whether the pattern contains `..`.
        has_rest: bool,
        /// Patterns after the `..`, if any.
        suffix: Vec<PatternId>,
    },
    /// A list of alternatives, written `left | right`.
    Or(Vec<PatternId>),
    /// Syntax the parser could not turn into a pattern.
    Error,
}

/// The field list of an enum variant pattern.
#[derive(Clone, Debug, PartialEq)]
pub enum VariantPatternFields {
    /// The variant is written without parentheses or braces.
    Unit,
    /// Positional fields, in source order.
    Positional(Vec<PatternId>),
    /// Named fields, in source order, with `..` optionally ignoring the rest.
    Named {
        /// The fields matched by the pattern, in source order.
        fields: Vec<NamedPatternField>,
        /// Whether the pattern ends with `..`, ignoring the other fields.
        has_rest: bool,
    },
}

/// One named field in a variant or struct pattern.
#[derive(Clone, Debug, PartialEq)]
pub struct NamedPatternField {
    /// The field name.
    pub name: Symbol,
    /// The span of the field name.
    pub name_span: Span,
    /// The pattern applied to the field.
    pub pattern: PatternId,
    /// The span from the field name through its pattern.
    pub span: Span,
}

/// One arm of a `match` expression.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchArm {
    /// The pattern that selects this arm.
    pub pattern: PatternId,
    /// An optional `if` guard, evaluated after the pattern matches.
    pub guard: Option<ExprId>,
    /// The expression evaluated when the pattern and guard succeed.
    pub body: ExprId,
    /// The source range of the whole arm.
    pub span: Span,
}
