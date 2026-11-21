//! Visibility and item modifiers.
//!
//! Modifiers are syntax only: the parser records what was written and rejects
//! combinations that have no meaning. Name resolution decides whether a
//! visibility is reachable from another module or package.

use seal_span::Span;
use seal_symbol::Symbol;

use crate::id::CfgExprId;

/// The declared visibility of an item, field, or method.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Visibility {
    /// Visible in the declaring module and its descendants.
    #[default]
    Private,
    /// Visible anywhere in the declaring package.
    Internal,
    /// Visible outside the package when the declaring module is exposed.
    Public,
}

impl Visibility {
    /// Returns the source spelling of the visibility.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Internal => "internal",
            Self::Public => "public",
        }
    }
}

/// Modifiers written before an item, field, or method.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// The declared visibility; private when the source omitted it.
    pub visibility: Visibility,
    /// Whether the source wrote a visibility modifier, including `private`.
    pub has_visibility: bool,
    /// A `cfg(...)` condition, when present.
    pub cfg: Option<CfgExprId>,
    /// Whether the item was declared with the contextual `test` modifier.
    pub is_test: bool,
    /// The ABI spelling after `export`, when the item is exported to C.
    pub export: Option<Symbol>,
    /// The span of the `export` modifier, for diagnostics.
    pub export_span: Option<Span>,
}

impl Modifiers {
    /// Creates the default modifiers for an item with no prefix.
    #[inline]
    pub const fn none() -> Self {
        Self {
            visibility: Visibility::Private,
            has_visibility: false,
            cfg: None,
            is_test: false,
            export: None,
            export_span: None,
        }
    }

    /// Returns `true` when no modifier was written.
    #[inline]
    pub const fn is_empty(self) -> bool {
        !self.has_visibility && self.cfg.is_none() && !self.is_test
    }
}
