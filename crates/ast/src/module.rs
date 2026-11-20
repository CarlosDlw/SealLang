//! Module declarations and imports.
//!
//! A module is a semantic namespace declared in the source, not a filesystem
//! convention. Imports are absolute and file-scoped; they are collected before
//! resolution so cycles between modules are legal.

use seal_span::Span;
use seal_symbol::Symbol;

/// The `module path::to::module;` declaration at the start of a file.
#[derive(Clone, Debug, PartialEq)]
pub struct ModuleDecl {
    /// The path segments, in source order.
    pub path: Vec<Symbol>,
    /// The span from the first segment through the last one.
    pub path_span: Span,
}

/// One `use` declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct UseDecl {
    /// The imported path.
    pub path: UsePath,
}

/// The first segment of an import path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UseRoot {
    /// The current package root, written `root`.
    Root,
    /// A top-level module or dependency alias.
    Name(Symbol),
}

impl UseRoot {
    /// Returns the root name, if the path does not start with `root`.
    #[inline]
    pub const fn name(self) -> Option<Symbol> {
        match self {
            Self::Root => None,
            Self::Name(name) => Some(name),
        }
    }
}

/// An absolute import path.
#[derive(Clone, Debug, PartialEq)]
pub struct UsePath {
    /// The first segment.
    pub root: UseRoot,
    /// The span of the first segment.
    pub root_span: Span,
    /// The remaining segments; a group is represented by [`UseSegment::Group`].
    pub segments: Vec<UseSegment>,
    /// The alias written after the whole path, as in `use a::b as c;`.
    pub alias: Option<Symbol>,
    /// The span of the whole-path alias.
    pub alias_span: Option<Span>,
    /// The span from the root through the alias or last segment.
    pub span: Span,
}

/// One segment of an import path.
#[derive(Clone, Debug, PartialEq)]
pub enum UseSegment {
    /// A plain name, such as `shapes` in `use geometry::shapes;`.
    Name {
        /// The segment name.
        name: Symbol,
        /// The span of the name.
        span: Span,
    },
    /// A braced group, such as `{Circle, Polygon as Poly}`.
    Group {
        /// The grouped imports, in source order.
        items: Vec<UseItem>,
        /// The span from `{` through `}`.
        span: Span,
    },
}

/// One name inside a braced import group.
#[derive(Clone, Debug, PartialEq)]
pub struct UseItem {
    /// The imported name.
    pub name: Symbol,
    /// The span of the name.
    pub name_span: Span,
    /// The local alias, when the source wrote `as`.
    pub alias: Option<Symbol>,
    /// The span of the alias.
    pub alias_span: Option<Span>,
    /// The span from the name through the alias.
    pub span: Span,
}
