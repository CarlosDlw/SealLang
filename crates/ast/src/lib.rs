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
mod cfg;
mod expr;
mod extern_item;
mod generics;
mod id;
mod item;
mod modifiers;
mod module;
mod pattern;
mod program;
mod ty;

pub use block::{Block, ForSource, LetStmt, Stmt, StmtKind};
pub use cfg::{CfgCmpOp, CfgExpr, CfgExprKind, CfgLiteral};
pub use expr::{
    AssignOp, BinaryOp, ClosureParam, Expr, ExprKind, FieldInit, Literal, UnaryOp, UpdateOp,
};
pub use extern_item::{ExternBlockDecl, ExternFnDecl, ExternStaticDecl, LinkDirective, LinkName};
pub use generics::{
    Bound, BoundKind, GenericArgs, GenericParamDecl, GenericParams, PathSegment, TraitPath,
    TypePath, WhereClause, WherePredicate,
};
pub use id::{
    BlockId, BoundId, CfgExprId, ExprId, FieldDeclId, GenericParamId, ItemId, ParamId, PatternId,
    StmtId, TupleFieldDeclId, TypeId, VariantDeclId,
};
pub use item::{
    ConstDecl, EnumDecl, FieldDecl, FnDecl, ImplDecl, Item, ItemKind, OpaqueTypeDecl, Param,
    StaticDecl, StructDecl, TraitDecl, TraitMethodDecl, TupleFieldDecl, TypeAliasDecl, VariantDecl,
    VariantFieldList,
};
pub use modifiers::{Modifiers, Visibility};
pub use module::{ModuleDecl, UseDecl, UseItem, UsePath, UseRoot, UseSegment};
pub use pattern::{
    MatchArm, NamedPatternField, Pattern, PatternKind, PatternLiteral, VariantPatternFields,
};
pub use program::{Ast, AstBuilder, Program};
pub use seal_def::Deprecation;
pub use ty::{PrimitiveType, Type, TypeKind};
