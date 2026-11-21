//! Items: the top level of a program.

use seal_span::Span;
use seal_symbol::Symbol;

use crate::extern_item::{ExternBlockDecl, ExternFnDecl, ExternStaticDecl};
use crate::generics::{GenericParams, TraitPath, WhereClause};
use crate::id::{
    BlockId, CfgExprId, ExprId, FieldDeclId, ItemId, ParamId, TupleFieldDeclId, TypeId,
    VariantDeclId,
};
use crate::modifiers::{Modifiers, Visibility};
use crate::module::{ModuleDecl, UseDecl};

/// A top-level item.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// What the item is.
    pub kind: ItemKind,
    /// Visibility, `cfg`, and test modifiers written before the item.
    pub modifiers: Modifiers,
    /// The source range of the outer doc comments attached to the item.
    pub docs: Option<Span>,
    /// The item's full source range.
    pub span: Span,
}

/// The kinds of top-level item.
#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    /// A module path declaration at the start of a file.
    Module(ModuleDecl),
    /// An absolute, file-scoped import.
    Use(UseDecl),
    /// A function declaration.
    Func(FnDecl),
    /// A constant declaration.
    Const(ConstDecl),
    /// A static declaration, immutable or mutable.
    Static(StaticDecl),
    /// A struct declaration.
    Struct(StructDecl),
    /// An opaque type declared by `extern type Name;`.
    OpaqueType(OpaqueTypeDecl),
    /// An enum declaration.
    Enum(EnumDecl),
    /// An inherent implementation block for a nominal struct, or a trait
    /// implementation for one.
    Impl(ImplDecl),
    /// A trait declaration.
    Trait(TraitDecl),
    /// One method signature inside a trait declaration.
    TraitMethod(TraitMethodDecl),
    /// A block of declarations sharing one ABI.
    ExternBlock(ExternBlockDecl),
    /// An external function declaration.
    ExternFunc(ExternFnDecl),
    /// An external static declaration.
    ExternStatic(ExternStaticDecl),
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
    /// Whether the declaration was written with `extern struct`.
    ///
    /// An external struct has the C representation: fields keep declaration
    /// order and C alignment, and the type is only a valid FFI boundary type
    /// when every field is FFI-safe.
    pub is_extern: bool,
    /// Declared type parameters.
    pub generics: GenericParams,
    /// Bounds written after the header.
    pub where_clause: WhereClause,
    /// Fields in declaration order.
    pub fields: Vec<FieldDeclId>,
}

/// An opaque type declared by `extern type Name;`.
///
/// The type has no size, alignment, fields, or destructor. It only appears
/// behind a raw pointer, where it names storage that C owns.
#[derive(Clone, Debug, PartialEq)]
pub struct OpaqueTypeDecl {
    /// The type name.
    pub name: Symbol,
    /// The span of the name, for diagnostics that point at it.
    pub name_span: Span,
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
    /// The source range of the outer doc comments attached to the variant.
    pub docs: Option<Span>,
    /// The declared field list.
    pub fields: VariantFieldList,
    /// A `cfg(...)` condition written on the variant, retained so the rejected
    /// syntax still has a reachable node.
    pub cfg: Option<CfgExprId>,
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
    /// The source range of the outer doc comments attached to the field.
    pub docs: Option<Span>,
    /// The declared field type.
    pub ty: TypeId,
    /// The field's declared visibility.
    pub visibility: Visibility,
    /// A `cfg(...)` condition written on the field, retained so the rejected
    /// syntax still has a reachable node.
    pub cfg: Option<CfgExprId>,
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
/// A trait is a list of method signatures, some of which may carry a default
/// body. Implementing the trait attaches one concrete function per signature,
/// so a call on a concrete type or a type parameter is static; an object-safe
/// trait can also be used behind `dyn`, where dispatch goes through a vtable. A
/// signature writes `Self` wherever the implementing type appears, including in
/// the receiver type.
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
/// A trait method either ends with `;`, leaving the body to each implementation,
/// or carries a default body that implementations may use unchanged. The parser
/// stores methods as items so the arena and its reachability invariants cover
/// them.
#[derive(Clone, Debug, PartialEq)]
pub struct TraitMethodDecl {
    /// The method name.
    pub name: Symbol,
    /// The span of the name, for diagnostics that point at it.
    pub name_span: Span,
    /// Whether the method is declared `unsafe func`.
    pub is_unsafe: bool,
    /// Declared method type parameters.
    pub generics: GenericParams,
    /// Parameters in source order, including the receiver when present.
    pub params: Vec<ParamId>,
    /// The declared return type; `None` means unit.
    pub ret: Option<TypeId>,
    /// Bounds written after the return type.
    pub where_clause: WhereClause,
    /// The default body, or `None` for a signature-only declaration.
    pub body: Option<BlockId>,
    /// A `cfg(...)` condition written on the method, retained so the rejected
    /// syntax still has a reachable node.
    pub cfg: Option<CfgExprId>,
}

/// A function declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct FnDecl {
    /// The function name.
    pub name: Symbol,
    /// The span of the name, for diagnostics that point at it.
    pub name_span: Span,
    /// Whether the declaration is a `const func`.
    pub is_const: bool,
    /// Whether the declaration is an `unsafe func`.
    pub is_unsafe: bool,
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

/// A constant declaration.
///
/// A constant has no storage of its own: its value is evaluated at compile time
/// and every use is replaced by that value. Both the type and the initializer
/// are required.
#[derive(Clone, Debug, PartialEq)]
pub struct ConstDecl {
    /// The constant name.
    pub name: Symbol,
    /// The span of the name, for diagnostics that point at it.
    pub name_span: Span,
    /// The declared type.
    pub ty: TypeId,
    /// The constant expression.
    pub init: ExprId,
}

/// A static declaration.
///
/// A static owns one allocation with program lifetime, initialized at compile
/// time. Only `static mut` is writable, and every access to it requires an
/// enclosing `unsafe` block.
#[derive(Clone, Debug, PartialEq)]
pub struct StaticDecl {
    /// The static name.
    pub name: Symbol,
    /// The span of the name, for diagnostics that point at it.
    pub name_span: Span,
    /// Whether the declaration is `static mut`.
    pub mutable: bool,
    /// The declared type.
    pub ty: TypeId,
    /// The initializer.
    pub init: ExprId,
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
