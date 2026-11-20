//! `extern` declarations.
//!
//! The AST keeps the ABI as an interned string: whether the spelling names a
//! convention is a semantic question, decided once during lowering. Parameters
//! and types reuse the ordinary nodes, so an external signature has exactly the
//! same shape as a Seal one.

use seal_span::Span;
use seal_symbol::Symbol;

use crate::id::{ItemId, ParamId, TypeId};

/// A `link "name";` directive inside an external block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkDirective {
    /// The library name passed to the linker.
    pub name: Symbol,
    /// The complete directive range.
    pub span: Span,
}

/// A block of declarations sharing one ABI.
#[derive(Clone, Debug, PartialEq)]
pub struct ExternBlockDecl {
    /// The ABI spelling.
    pub abi: Symbol,
    /// The span of the ABI string, for diagnostics.
    pub abi_span: Span,
    /// The library directives in source order.
    pub links: Vec<LinkDirective>,
    /// The declarations in source order.
    pub items: Vec<ItemId>,
}

/// A `link_name` written after an external function's return type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkName {
    /// The linker symbol.
    pub symbol: Symbol,
    /// The span of the string literal.
    pub span: Span,
}

/// An external function declaration.
///
/// The declaration has no body and no generic parameters. `variadic` records a
/// trailing `...`; the parser accepts the syntax so a later phase can implement
/// it, but this phase rejects the declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct ExternFnDecl {
    /// The ABI spelling.
    pub abi: Symbol,
    /// The span of the ABI string.
    pub abi_span: Span,
    /// The name Seal uses to call the function.
    pub name: Symbol,
    /// The span of the declared name.
    pub name_span: Span,
    /// Parameters in source order.
    pub params: Vec<ParamId>,
    /// The declared return type; `None` means `void`.
    pub ret: Option<TypeId>,
    /// The linker symbol, when it differs from the declared name.
    pub link_name: Option<LinkName>,
    /// Whether the declaration ends with `...`.
    pub variadic: bool,
    /// The complete declaration range.
    pub span: Span,
}

/// An external static declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct ExternStaticDecl {
    /// The ABI spelling.
    pub abi: Symbol,
    /// The span of the ABI string.
    pub abi_span: Span,
    /// The name Seal uses to read or write the static.
    pub name: Symbol,
    /// The span of the declared name.
    pub name_span: Span,
    /// Whether the declaration is `static mut`.
    pub mutable: bool,
    /// The declared storage type.
    pub ty: TypeId,
    /// The complete declaration range.
    pub span: Span,
}
