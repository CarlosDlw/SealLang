//! The tree owner and the builder the parser uses.

use seal_index::IndexVec;
use seal_span::Span;
use seal_symbol::Symbol;

use crate::block::{Block, LetStmt, Stmt, StmtKind};
use crate::expr::{Expr, ExprKind};
use crate::id::{BlockId, ExprId, ItemId, ParamId, StmtId, TypeId};
use crate::item::{Item, ItemKind, Param};
use crate::ty::{Type, TypeKind};

/// The items of one source file, in source order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Program {
    items: Vec<ItemId>,
}

impl Program {
    /// Returns the item IDs in source order.
    #[inline]
    pub fn items(&self) -> &[ItemId] {
        &self.items
    }

    /// Returns the number of items.
    #[inline]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns `true` if the file declared no items.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// The syntax tree of one file.
///
/// IDs are dense indices into the node arenas. They are only meaningful for the
/// tree that produced them; passing an ID to another tree is a programming
/// error and panics.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ast {
    items: IndexVec<ItemId, Item>,
    params: IndexVec<ParamId, Param>,
    blocks: IndexVec<BlockId, Block>,
    stmts: IndexVec<StmtId, Stmt>,
    exprs: IndexVec<ExprId, Expr>,
    types: IndexVec<TypeId, Type>,
    program: Program,
}

impl Ast {
    /// Returns the parsed program.
    #[inline]
    pub fn program(&self) -> &Program {
        &self.program
    }

    /// Returns an item.
    #[inline]
    pub fn item(&self, id: ItemId) -> &Item {
        &self.items[id]
    }

    /// Returns a parameter.
    #[inline]
    pub fn param(&self, id: ParamId) -> &Param {
        &self.params[id]
    }

    /// Returns a block.
    #[inline]
    pub fn block(&self, id: BlockId) -> &Block {
        &self.blocks[id]
    }

    /// Returns a statement.
    #[inline]
    pub fn stmt(&self, id: StmtId) -> &Stmt {
        &self.stmts[id]
    }

    /// Returns an expression.
    #[inline]
    pub fn expr(&self, id: ExprId) -> &Expr {
        &self.exprs[id]
    }

    /// Returns a type.
    #[inline]
    pub fn ty(&self, id: TypeId) -> &Type {
        &self.types[id]
    }

    /// Returns all item nodes, indexable by [`ItemId`].
    #[inline]
    pub fn item_arena(&self) -> &IndexVec<ItemId, Item> {
        &self.items
    }

    /// Returns all parameter nodes, indexable by [`ParamId`].
    #[inline]
    pub fn param_arena(&self) -> &IndexVec<ParamId, Param> {
        &self.params
    }

    /// Returns all block nodes, indexable by [`BlockId`].
    #[inline]
    pub fn block_arena(&self) -> &IndexVec<BlockId, Block> {
        &self.blocks
    }

    /// Returns all statement nodes, indexable by [`StmtId`].
    #[inline]
    pub fn stmt_arena(&self) -> &IndexVec<StmtId, Stmt> {
        &self.stmts
    }

    /// Returns all expression nodes, indexable by [`ExprId`].
    #[inline]
    pub fn expr_arena(&self) -> &IndexVec<ExprId, Expr> {
        &self.exprs
    }

    /// Returns all type nodes, indexable by [`TypeId`].
    #[inline]
    pub fn type_arena(&self) -> &IndexVec<TypeId, Type> {
        &self.types
    }
}

/// Builds an [`Ast`].
///
/// The parser is the only caller; the builder is public so alternative front
/// ends (a formatter's reparse, a test fixture) can construct trees without
/// duplicating the node layout. Programs are attached in [`AstBuilder::finish`].
#[derive(Clone, Debug, Default)]
pub struct AstBuilder {
    ast: Ast,
}

impl AstBuilder {
    /// Creates an empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an item and returns its ID.
    pub fn push_item(&mut self, kind: ItemKind, span: Span) -> ItemId {
        self.ast.items.push(Item { kind, span })
    }

    /// Adds a parameter and returns its ID.
    pub fn push_param(&mut self, name: Symbol, name_span: Span, ty: TypeId, span: Span) -> ParamId {
        self.ast.params.push(Param {
            name,
            name_span,
            ty,
            span,
        })
    }

    /// Adds a block and returns its ID.
    pub fn push_block(
        &mut self,
        span: Span,
        statements: Vec<StmtId>,
        tail: Option<ExprId>,
    ) -> BlockId {
        self.ast.blocks.push(Block {
            statements,
            tail,
            span,
        })
    }

    /// Adds a `let` statement and returns its ID.
    pub fn push_let(
        &mut self,
        span: Span,
        name: Symbol,
        name_span: Span,
        ty: Option<TypeId>,
        init: Option<ExprId>,
    ) -> StmtId {
        self.ast.stmts.push(Stmt {
            kind: StmtKind::Let(LetStmt {
                name,
                name_span,
                ty,
                init,
            }),
            span,
        })
    }

    /// Adds a `return` statement and returns its ID.
    pub fn push_return(&mut self, span: Span, value: Option<ExprId>) -> StmtId {
        self.ast.stmts.push(Stmt {
            kind: StmtKind::Return(value),
            span,
        })
    }

    /// Adds an expression statement and returns its ID.
    pub fn push_expr_stmt(&mut self, span: Span, expr: ExprId) -> StmtId {
        self.ast.stmts.push(Stmt {
            kind: StmtKind::Expr(expr),
            span,
        })
    }

    /// Adds an error statement and returns its ID.
    pub fn push_error_stmt(&mut self, span: Span) -> StmtId {
        self.ast.stmts.push(Stmt {
            kind: StmtKind::Error,
            span,
        })
    }

    /// Adds an expression and returns its ID.
    pub fn push_expr(&mut self, kind: ExprKind, span: Span) -> ExprId {
        self.ast.exprs.push(Expr { kind, span })
    }

    /// Adds a type and returns its ID.
    pub fn push_type(&mut self, kind: TypeKind, span: Span) -> TypeId {
        self.ast.types.push(Type { kind, span })
    }

    /// Returns the partially built tree.
    #[inline]
    pub fn ast(&self) -> &Ast {
        &self.ast
    }

    /// Attaches the program's items and returns the finished tree.
    pub fn finish(mut self, items: Vec<ItemId>) -> Ast {
        self.ast.program = Program { items };
        self.ast
    }
}
