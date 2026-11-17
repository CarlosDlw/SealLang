//! Items: the top level of a program.

use seal_span::Span;
use seal_symbol::Symbol;

use crate::id::{BlockId, FieldDeclId, ItemId, ParamId, TypeId, VariantDeclId};

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
    /// An enum declaration.
    Enum(EnumDecl),
    /// An inherent implementation block for a nominal struct, or a trait
    /// implementation for one.
    Impl(ImplDecl),
    /// A trait declaration.
    Trait(TraitDecl),
    /// One method signature inside a trait declaration.
    TraitMethod(TraitMethodDecl),
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

/// An enum declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct EnumDecl {
    /// The enum name.
    pub name: Symbol,
    /// The span of the name, for diagnostics that point at it.
    pub name_span: Span,
    /// Variants in declaration order (also discriminant order).
    pub variants: Vec<VariantDeclId>,
}

/// One variant in an enum declaration.
///
/// A variant either carries no fields (`Variant`) or a list of named fields
/// (`Variant { name: Type, ... }`). Seal has no tuple variants, so the field
/// list is always the braced form.
#[derive(Clone, Debug, PartialEq)]
pub struct VariantDecl {
    /// The variant name.
    pub name: Symbol,
    /// The span of the variant name.
    pub name_span: Span,
    /// Fields in declaration order; empty for a unit variant.
    pub fields: Vec<FieldDeclId>,
    /// The span from the variant name through the closing brace.
    pub span: Span,
}

/// One field in a struct or variant declaration.
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

/// An implementation block.
///
/// `impl Type { ... }` attaches functions directly to a nominal struct, while
/// `impl Trait for Type { ... }` implements a trait declared in the same file.
/// A function whose first parameter is named `self` is an instance method;
/// every other function is an associated function.
#[derive(Clone, Debug, PartialEq)]
pub struct ImplDecl {
    /// The struct receiving the functions.
    pub type_name: Symbol,
    /// The span of the type name, for diagnostics.
    pub type_span: Span,
    /// The trait being implemented, when the block has the `Trait for Type`
    /// form. It holds the trait name and the span of that name.
    pub trait_name: Option<(Symbol, Span)>,
    /// The functions declared in source order.
    pub functions: Vec<ItemId>,
}

/// A trait declaration.
///
/// A trait is a list of method signatures. Seal resolves every call statically:
/// implementing the trait attaches one concrete function per signature, and
/// there is no vtable or trait object. A signature writes `Self` wherever the
/// implementing type appears, including in the receiver type.
#[derive(Clone, Debug, PartialEq)]
pub struct TraitDecl {
    /// The trait name.
    pub name: Symbol,
    /// The span of the name, for diagnostics that point at it.
    pub name_span: Span,
    /// Method signatures in declaration order.
    pub methods: Vec<ItemId>,
}

/// One method signature inside a trait declaration.
///
/// Trait methods declare no body: the implementation supplies it. The parser
/// stores them as items so the arena and its reachability invariants cover them.
#[derive(Clone, Debug, PartialEq)]
pub struct TraitMethodDecl {
    /// The method name.
    pub name: Symbol,
    /// The span of the name, for diagnostics that point at it.
    pub name_span: Span,
    /// Parameters in source order, including the receiver when present.
    pub params: Vec<ParamId>,
    /// The declared return type; `None` means unit.
    pub ret: Option<TypeId>,
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
