//! The Seal abstract syntax tree.
//!
//! Nodes live in typed arenas ([`IndexVec`](seal_index::IndexVec)) and refer to
//! each other through [`ItemId`], [`BlockId`], [`StmtId`], [`ExprId`],
//! [`TypeId`], and [`ParamId`]. IDs are four-byte, copyable handles: later phases
//! can keep side tables keyed by them without borrowing the tree, and adding an
//! analysis does not require another lifetime parameter.
//!
//! Every node carries its source [`Span`](seal_span::Span). Identifiers and
//! literal payloads are interned [`Symbol`](seal_symbol::Symbol)s. The parser
//! builds the tree through [`AstBuilder`], which is the only type that can add
//! nodes; consumers only read.
//!
//! # Partial trees
//!
//! Parsing is resilient: an invalid construct becomes an `Error` node with the
//! span of the offending syntax, so consumers always see a complete tree with
//! valid IDs. The driver stops after syntax errors, but an editor can traverse
//! the partial tree to provide highlighting and navigation around errors.

#![warn(missing_docs)]

mod block;
mod expr;
mod id;
mod item;
mod program;
mod ty;

pub use block::{Block, LetStmt, Stmt, StmtKind};
pub use expr::{AssignOp, BinaryOp, Expr, ExprKind, FieldInit, Literal, UnaryOp, UpdateOp};
pub use id::{BlockId, ExprId, FieldDeclId, ItemId, ParamId, StmtId, TypeId, VariantDeclId};
pub use item::{
    EnumDecl, FieldDecl, FnDecl, ImplDecl, Item, ItemKind, Param, StructDecl, TraitDecl,
    TraitMethodDecl, VariantDecl,
};
pub use program::{Ast, AstBuilder, Program};
pub use ty::{PrimitiveType, Type, TypeKind};
