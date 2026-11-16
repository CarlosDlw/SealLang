//! The Seal high-level intermediate representation.
//!
//! Lowering turns the parser's [`Ast`](seal_ast::Ast) into a tree where every
//! name is resolved and every syntactic convenience is gone: parentheses are
//! dropped, and each function owns exactly one [`Body`].
//!
//! The result is total. A name that does not resolve, or a `break` outside a
//! loop, produces a diagnostic (`E2xxx`) and an `Error` node, so consumers can
//! keep walking without special cases.
//!
//! # Resolution
//!
//! Functions are collected before any body is lowered, so a function can call
//! one declared later. Inside a body, names resolve through a stack of ribs:
//! the innermost block wins, `let` bindings shadow parameters and outer
//! bindings, and a binding is not visible in its own initializer. Parameters and
//! bindings become [`DefId`](seal_def::DefId)s carrying their mutability, which
//! later phases use instead of re-reading the source.
//!
//! # Example
//!
//! ```ignore
//! let ast = seal_parser::parse(&lexed, session.diag(), session.interner());
//! let hir = seal_hir::lower(&ast, session.diag(), session.interner());
//! ```

#![warn(missing_docs)]

mod body;
mod expr;
mod id;
mod lower;
mod primitives;
mod res;

#[cfg(test)]
mod tests;

pub use body::{Block, Body, ParamData, Stmt, StmtKind};
pub use expr::{Expr, ExprKind, Place, Projection};
pub use id::{BodyId, HirBlockId, HirExprId, HirStmtId};
pub use primitives::primitive_kind;
pub use res::Res;

use seal_ast::Ast;
use seal_def::{DefData, DefId, Definitions};
use seal_diagnostics::DiagCtxt;
use seal_hash::FxHashMap;
use seal_index::IndexVec;
use seal_symbol::Interner;
use seal_ty::{AdtId, TyCtxt};

/// The lowered representation of one file.
#[derive(Debug)]
pub struct Hir {
    definitions: Definitions,
    bodies: IndexVec<BodyId, Body>,
    blocks: IndexVec<HirBlockId, Block>,
    stmts: IndexVec<HirStmtId, Stmt>,
    exprs: IndexVec<HirExprId, Expr>,
    ty_ctxt: TyCtxt,
    body_of: FxHashMap<DefId, BodyId>,
    structs: FxHashMap<DefId, AdtId>,
}

impl Hir {
    /// Lowers a parsed file, reporting name-resolution errors through `dcx`.
    ///
    /// The returned tree is always complete; syntax or names that failed are
    /// represented by `Error` nodes after their diagnostic was emitted.
    pub fn lower(ast: &Ast, dcx: &DiagCtxt, interner: &Interner) -> Self {
        lower::lower_ast(ast, dcx, interner)
    }

    /// Returns the definition table.
    pub fn definitions(&self) -> &Definitions {
        &self.definitions
    }

    /// Returns the type context that owns every lowered type.
    pub fn ty_ctxt(&self) -> &TyCtxt {
        &self.ty_ctxt
    }

    /// Returns all bodies, indexable by [`BodyId`].
    pub fn bodies(&self) -> &IndexVec<BodyId, Body> {
        &self.bodies
    }

    /// Returns a body.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this tree.
    pub fn body(&self, id: BodyId) -> &Body {
        &self.bodies[id]
    }

    /// Returns the ID of the body owned by `owner`, if it has one.
    pub fn body_id_of(&self, owner: DefId) -> Option<BodyId> {
        self.body_of.get(&owner).copied()
    }

    /// Returns the body owned by `owner`, if it has one.
    pub fn body_of(&self, owner: DefId) -> Option<&Body> {
        self.body_id_of(owner).map(|id| &self.bodies[id])
    }

    /// Returns a block.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this tree.
    pub fn block(&self, id: HirBlockId) -> &Block {
        &self.blocks[id]
    }

    /// Returns a statement.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this tree.
    pub fn stmt(&self, id: HirStmtId) -> &Stmt {
        &self.stmts[id]
    }

    /// Returns an expression.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this tree.
    pub fn expr(&self, id: HirExprId) -> &Expr {
        &self.exprs[id]
    }

    /// Returns the place denoted by `expr`, when it denotes storage.
    ///
    /// A local path is a place rooted at its definition; `*reference` is the
    /// place reached by one dereference; a field access projects from its base
    /// place. Borrows, calls, and literals are not places. Assignment and update
    /// targets are expressions, so this is how a consumer asks whether the
    /// target is assignable without matching on the tree.
    pub fn place(&self, expr: HirExprId) -> Option<Place> {
        match self.exprs[expr].kind {
            ExprKind::Path(Res::Local(def)) => Some(Place::local(def)),
            ExprKind::Deref { operand } => self.place(operand).map(Place::deref),
            ExprKind::Field { base, name, .. } => self.place(base).map(|place| place.field(name)),
            _ => None,
        }
    }

    /// Returns the aggregate reserved for the struct definition `def`.
    pub fn adt(&self, def: DefId) -> Option<AdtId> {
        self.structs.get(&def).copied()
    }

    pub(crate) fn insert_struct(&mut self, def: DefId, adt: AdtId) {
        self.structs.insert(def, adt);
    }

    /// Returns all blocks, indexable by [`HirBlockId`].
    pub fn block_arena(&self) -> &IndexVec<HirBlockId, Block> {
        &self.blocks
    }

    /// Returns all statements, indexable by [`HirStmtId`].
    pub fn stmt_arena(&self) -> &IndexVec<HirStmtId, Stmt> {
        &self.stmts
    }

    /// Returns all expressions, indexable by [`HirExprId`].
    pub fn expr_arena(&self) -> &IndexVec<HirExprId, Expr> {
        &self.exprs
    }

    pub(crate) fn empty(mut ty_ctxt: TyCtxt) -> Self {
        lower::intern_primitives(&mut ty_ctxt);
        Self {
            definitions: Definitions::new(),
            bodies: IndexVec::new(),
            blocks: IndexVec::new(),
            stmts: IndexVec::new(),
            exprs: IndexVec::new(),
            ty_ctxt,
            body_of: FxHashMap::default(),
            structs: FxHashMap::default(),
        }
    }

    pub(crate) fn define(&mut self, data: DefData) -> DefId {
        self.definitions.push(data)
    }

    pub(crate) fn push_body(&mut self, body: Body) -> BodyId {
        self.bodies.push(body)
    }

    pub(crate) fn set_body_of(&mut self, owner: DefId, body: BodyId) {
        self.body_of.insert(owner, body);
    }

    pub(crate) fn push_block(&mut self, block: Block) -> HirBlockId {
        self.blocks.push(block)
    }

    pub(crate) fn push_stmt(&mut self, stmt: Stmt) -> HirStmtId {
        self.stmts.push(stmt)
    }

    pub(crate) fn push_expr(&mut self, expr: Expr) -> HirExprId {
        self.exprs.push(expr)
    }

    pub(crate) fn ty_ctxt_mut(&mut self) -> &mut TyCtxt {
        &mut self.ty_ctxt
    }
}

/// Lowers a parsed file, reporting name-resolution errors through `dcx`.
pub fn lower(ast: &Ast, dcx: &DiagCtxt, interner: &Interner) -> Hir {
    Hir::lower(ast, dcx, interner)
}
