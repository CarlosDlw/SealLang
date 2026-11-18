//! Items: the top level of a program.

use seal_span::Span;
use seal_symbol::Symbol;

use crate::generics::{GenericParams, TraitPath, WhereClause};
use crate::id::{BlockId, FieldDeclId, ItemId, ParamId, TupleFieldDeclId, TypeId, VariantDeclId};

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
    /// Declared type parameters.
    pub generics: GenericParams,
    /// Bounds written after the header.
    pub where_clause: WhereClause,
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
    /// Declared type parameters.
    pub generics: GenericParams,
    /// Bounds written after the header.
    pub where_clause: WhereClause,
    /// Variants in declaration order (also discriminant order).
    pub variants: Vec<VariantDeclId>,
}

/// One variant in an enum declaration.
///
/// A variant carries no fields (`Variant`), positional fields
/// (`Variant(Type, ...)`), or named fields (`Variant { name: Type, ... }`).
/// The three forms are exclusive: mixing positional and named fields would make
/// construction and pattern matching depend on two orders, so the parser
/// rejects it.
#[derive(Clone, Debug, PartialEq)]
pub struct VariantDecl {
    /// The variant name.
    pub name: Symbol,
    /// The span of the variant name.
    pub name_span: Span,
    /// The declared field list.
    pub fields: VariantFieldList,
    /// The span from the variant name through the closing delimiter.
    pub span: Span,
}

/// The fields declared by one enum variant.
#[derive(Clone, Debug, PartialEq)]
pub enum VariantFieldList {
    /// The variant carries no payload.
    Unit,
    /// Positional fields, in declaration order.
    Tuple(Vec<TupleFieldDeclId>),
    /// Named fields, in declaration order.
    Named(Vec<FieldDeclId>),
}

/// One positional field in an enum variant declaration.
///
/// The field has no name; its position in the variant is its identity. The
/// type's span is also the field's span, because there is no name to point at.
#[derive(Clone, Debug, PartialEq)]
pub struct TupleFieldDecl {
    /// The declared field type.
    pub ty: TypeId,
    /// The span of the field type.
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
    /// Declared type parameters.
    pub generics: GenericParams,
    /// The type receiving the functions.
    pub self_ty: TypeId,
    /// The trait being implemented, when the block has the `Trait for Type`
    /// form.
    pub trait_ref: Option<TraitPath>,
    /// Bounds written after the header.
    pub where_clause: WhereClause,
    /// The functions declared in source order.
    pub functions: Vec<ItemId>,
}

/// A trait declaration.
///
/// A trait is a list of method signatures. Implementing the trait attaches one
/// concrete function per signature, so a call on a concrete type or a type
/// parameter is static; an object-safe trait can also be used behind `dyn`,
/// where dispatch goes through a vtable. A signature writes `Self` wherever the
/// implementing type appears, including in the receiver type.
#[derive(Clone, Debug, PartialEq)]
pub struct TraitDecl {
    /// The trait name.
    pub name: Symbol,
    /// The span of the name, for diagnostics that point at it.
    pub name_span: Span,
    /// Declared type parameters.
    pub generics: GenericParams,
    /// Traits every implementor must also implement, in source order.
    pub supertraits: Vec<TraitPath>,
    /// Bounds written after the header.
    pub where_clause: WhereClause,
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
    /// Declared method type parameters.
    pub generics: GenericParams,
    /// Parameters in source order, including the receiver when present.
    pub params: Vec<ParamId>,
    /// The declared return type; `None` means unit.
    pub ret: Option<TypeId>,
    /// Bounds written after the return type.
    pub where_clause: WhereClause,
}

/// A function declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct FnDecl {
    /// The function name.
    pub name: Symbol,
    /// The span of the name, for diagnostics that point at it.
    pub name_span: Span,
    /// Declared function type parameters.
    pub generics: GenericParams,
    /// Parameters in source order.
    pub params: Vec<ParamId>,
    /// The declared return type; `None` means unit.
    pub ret: Option<TypeId>,
    /// Bounds written after the return type.
    pub where_clause: WhereClause,
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
