//! Items: the top level of a program.

use seal_span::Span;
use seal_symbol::Symbol;

use crate::id::{BlockId, ParamId, TypeId};

/// A top-level item.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// What the item is.
    pub kind: ItemKind,
    /// The item's full source range.
    pub span: Span,
}

/// The kinds of top-level item.
#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    /// A function declaration.
    Func(FnDecl),
    /// Syntax the parser could not turn into an item.
    Error,
}

/// A function declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct FnDecl {
    /// The function name.
    pub name: Symbol,
    /// The span of the name, for diagnostics that point at it.
    pub name_span: Span,
    /// Parameters in source order.
    pub params: Vec<ParamId>,
    /// The declared return type; `None` means unit.
    pub ret: Option<TypeId>,
    /// The function body.
    pub body: BlockId,
}

/// A function parameter.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    /// The parameter name.
    pub name: Symbol,
    /// The span of the name.
    pub name_span: Span,
    /// The declared parameter type.
    pub ty: TypeId,
    /// The span from the name through the type.
    pub span: Span,
}
