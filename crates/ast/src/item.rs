//! Items: the top level of a program.

use seal_span::Span;
use seal_symbol::Symbol;

use crate::id::{BlockId, FieldDeclId, ItemId, ParamId, TypeId};

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
    /// A struct declaration.
    Struct(StructDecl),
    /// An inherent implementation block for a nominal struct.
    Impl(ImplDecl),
    /// Syntax the parser could not turn into an item.
    Error,
}

/// A struct declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct StructDecl {
    /// The struct name.
    pub name: Symbol,
    /// The span of the name, for diagnostics that point at it.
    pub name_span: Span,
    /// Fields in declaration order.
    pub fields: Vec<FieldDeclId>,
}

/// One field in a struct declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldDecl {
    /// The field name.
    pub name: Symbol,
    /// The span of the field name.
    pub name_span: Span,
    /// The declared field type.
    pub ty: TypeId,
    /// The span from the name through the type.
    pub span: Span,
}

/// An inherent implementation block.
///
/// Seal has no traits yet, so an implementation only attaches functions to one
/// nominal struct. A function whose first parameter is named `self` is an
/// instance method; every other function is an associated function.
#[derive(Clone, Debug, PartialEq)]
pub struct ImplDecl {
    /// The struct receiving the functions.
    pub type_name: Symbol,
    /// The span of the type name, for diagnostics.
    pub type_span: Span,
    /// The functions declared in source order.
    pub functions: Vec<ItemId>,
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
