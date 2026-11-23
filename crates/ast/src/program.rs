//! The tree owner and the builder the parser uses.

use seal_index::IndexVec;
use seal_span::Span;
use seal_symbol::Symbol;

use crate::block::{Block, LetStmt, Stmt, StmtKind};
use crate::cfg::{CfgExpr, CfgExprKind};
use crate::expr::{Expr, ExprKind};
use crate::generics::{Bound, GenericParamDecl};
use crate::id::{
    BlockId, BoundId, CfgExprId, ExprId, FieldDeclId, GenericParamId, ItemId, ParamId, PatternId,
    StmtId, TupleFieldDeclId, TypeId, VariantDeclId,
};
use crate::item::{
    ConstDecl, FieldDecl, Item, ItemKind, Param, StaticDecl, TupleFieldDecl, VariantDecl,
    VariantFieldList,
};
use crate::modifiers::{Modifiers, Visibility};
use crate::module::{ModuleDecl, UseDecl};
use crate::pattern::Pattern;
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
    module_docs: Option<Span>,
    generic_params: IndexVec<GenericParamId, GenericParamDecl>,
    bounds: IndexVec<BoundId, Bound>,
    params: IndexVec<ParamId, Param>,
    fields: IndexVec<FieldDeclId, FieldDecl>,
    cfg_exprs: IndexVec<CfgExprId, CfgExpr>,
    tuple_fields: IndexVec<TupleFieldDeclId, TupleFieldDecl>,
    variants: IndexVec<VariantDeclId, VariantDecl>,
    blocks: IndexVec<BlockId, Block>,
    stmts: IndexVec<StmtId, Stmt>,
    exprs: IndexVec<ExprId, Expr>,
    patterns: IndexVec<PatternId, Pattern>,
    types: IndexVec<TypeId, Type>,
    program: Program,
}

impl Ast {
    /// Returns the inner doc comments (`//!`) for a file without an explicit
    /// module declaration.
    #[inline]
    pub fn module_docs(&self) -> Option<Span> {
        self.module_docs
    }

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

    /// Returns a generic parameter declaration.
    #[inline]
    pub fn generic_param(&self, id: GenericParamId) -> &GenericParamDecl {
        &self.generic_params[id]
    }

    /// Returns a bound.
    #[inline]
    pub fn bound(&self, id: BoundId) -> &Bound {
        &self.bounds[id]
    }

    /// Returns a parameter.
    #[inline]
    pub fn param(&self, id: ParamId) -> &Param {
        &self.params[id]
    }

    /// Returns a struct field declaration.
    #[inline]
    pub fn field(&self, id: FieldDeclId) -> &FieldDecl {
        &self.fields[id]
    }

    /// Returns an enum variant declaration.
    #[inline]
    pub fn variant(&self, id: VariantDeclId) -> &VariantDecl {
        &self.variants[id]
    }

    /// Returns a parsed `cfg(...)` expression.
    #[inline]
    pub fn cfg_expr(&self, id: CfgExprId) -> &CfgExpr {
        &self.cfg_exprs[id]
    }

    /// Returns a positional field of an enum variant.
    #[inline]
    pub fn tuple_field(&self, id: TupleFieldDeclId) -> &TupleFieldDecl {
        &self.tuple_fields[id]
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

    /// Returns a pattern.
    #[inline]
    pub fn pattern(&self, id: PatternId) -> &Pattern {
        &self.patterns[id]
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

    /// Returns all generic parameter nodes, indexable by [`GenericParamId`].
    #[inline]
    pub fn generic_param_arena(&self) -> &IndexVec<GenericParamId, GenericParamDecl> {
        &self.generic_params
    }

    /// Returns all bound nodes, indexable by [`BoundId`].
    #[inline]
    pub fn bound_arena(&self) -> &IndexVec<BoundId, Bound> {
        &self.bounds
    }

    /// Returns all parameter nodes, indexable by [`ParamId`].
    #[inline]
    pub fn param_arena(&self) -> &IndexVec<ParamId, Param> {
        &self.params
    }

    /// Returns all field declaration nodes, indexable by [`FieldDeclId`].
    #[inline]
    pub fn field_arena(&self) -> &IndexVec<FieldDeclId, FieldDecl> {
        &self.fields
    }

    /// Returns all enum variant declaration nodes, indexable by
    /// [`VariantDeclId`].
    #[inline]
    pub fn variant_arena(&self) -> &IndexVec<VariantDeclId, VariantDecl> {
        &self.variants
    }

    /// Returns all `cfg(...)` expression nodes, indexable by [`CfgExprId`].
    #[inline]
    pub fn cfg_expr_arena(&self) -> &IndexVec<CfgExprId, CfgExpr> {
        &self.cfg_exprs
    }

    /// Returns all positional variant field nodes, indexable by
    /// [`TupleFieldDeclId`].
    #[inline]
    pub fn tuple_field_arena(&self) -> &IndexVec<TupleFieldDeclId, TupleFieldDecl> {
        &self.tuple_fields
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

    /// Returns all pattern nodes, indexable by [`PatternId`].
    #[inline]
    pub fn pattern_arena(&self) -> &IndexVec<PatternId, Pattern> {
        &self.patterns
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
        self.push_item_with_modifiers(kind, span, Modifiers::none())
    }

    /// Adds an item with modifiers and returns its ID.
    pub fn push_item_with_modifiers(
        &mut self,
        kind: ItemKind,
        span: Span,
        modifiers: Modifiers,
    ) -> ItemId {
        self.ast.items.push(Item {
            kind,
            modifiers,
            docs: None,
            span,
        })
    }

    /// Attaches outer doc comments to an item.
    pub fn set_item_docs(&mut self, id: ItemId, docs: Option<Span>) {
        self.ast.items[id].docs = docs;
    }

    /// Adds a module declaration and returns its ID.
    pub fn push_module_item(
        &mut self,
        span: Span,
        declaration: ModuleDecl,
        modifiers: Modifiers,
    ) -> ItemId {
        self.push_item_with_modifiers(ItemKind::Module(declaration), span, modifiers)
    }

    /// Adds an import and returns its ID.
    pub fn push_use_item(
        &mut self,
        span: Span,
        declaration: UseDecl,
        modifiers: Modifiers,
    ) -> ItemId {
        self.push_item_with_modifiers(ItemKind::Use(declaration), span, modifiers)
    }

    /// Adds a constant declaration item and returns its ID.
    pub fn push_const_item(&mut self, span: Span, declaration: ConstDecl) -> ItemId {
        self.push_const_item_with_modifiers(span, declaration, Modifiers::none())
    }

    /// Adds a constant declaration with modifiers and returns its ID.
    pub fn push_const_item_with_modifiers(
        &mut self,
        span: Span,
        declaration: ConstDecl,
        modifiers: Modifiers,
    ) -> ItemId {
        self.push_item_with_modifiers(ItemKind::Const(declaration), span, modifiers)
    }

    /// Adds a static declaration item and returns its ID.
    pub fn push_static_item(&mut self, span: Span, declaration: StaticDecl) -> ItemId {
        self.push_static_item_with_modifiers(span, declaration, Modifiers::none())
    }

    /// Adds a static declaration with modifiers and returns its ID.
    pub fn push_static_item_with_modifiers(
        &mut self,
        span: Span,
        declaration: StaticDecl,
        modifiers: Modifiers,
    ) -> ItemId {
        self.push_item_with_modifiers(ItemKind::Static(declaration), span, modifiers)
    }

    /// Adds a parsed `cfg(...)` expression and returns its ID.
    pub fn push_cfg_expr(&mut self, kind: CfgExprKind, span: Span) -> CfgExprId {
        self.ast.cfg_exprs.push(CfgExpr { kind, span })
    }

    /// Returns a parsed `cfg(...)` expression.
    #[inline]
    pub fn cfg_expr(&self, id: CfgExprId) -> &CfgExpr {
        self.ast.cfg_expr(id)
    }

    /// Adds a generic parameter declaration and returns its ID.
    pub fn push_generic_param(
        &mut self,
        name: Symbol,
        name_span: Span,
        bounds: Vec<BoundId>,
        span: Span,
    ) -> GenericParamId {
        self.ast.generic_params.push(GenericParamDecl {
            name,
            name_span,
            bounds,
            span,
        })
    }

    /// Adds a bound and returns its ID.
    pub fn push_bound(&mut self, kind: crate::generics::BoundKind, span: Span) -> BoundId {
        self.ast.bounds.push(Bound { kind, span })
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

    /// Adds a struct field declaration and returns its ID.
    pub fn push_field(
        &mut self,
        name: Symbol,
        name_span: Span,
        ty: TypeId,
        span: Span,
    ) -> FieldDeclId {
        self.push_field_with_visibility(name, name_span, ty, span, Visibility::Private)
    }

    /// Adds a struct field with a declared visibility and returns its ID.
    pub fn push_field_with_visibility(
        &mut self,
        name: Symbol,
        name_span: Span,
        ty: TypeId,
        span: Span,
        visibility: Visibility,
    ) -> FieldDeclId {
        self.ast.fields.push(FieldDecl {
            name,
            name_span,
            docs: None,
            ty,
            visibility,
            cfg: None,
            deprecated: None,
            span,
        })
    }

    /// Adds a struct field with its parsed declaration modifiers.
    pub fn push_field_with_modifiers(
        &mut self,
        name: Symbol,
        name_span: Span,
        ty: TypeId,
        span: Span,
        modifiers: Modifiers,
    ) -> FieldDeclId {
        self.ast.fields.push(FieldDecl {
            name,
            name_span,
            docs: None,
            ty,
            visibility: modifiers.visibility,
            cfg: modifiers.cfg,
            deprecated: modifiers.deprecated,
            span,
        })
    }

    /// Attaches outer doc comments to a struct or variant field.
    pub fn set_field_docs(&mut self, id: FieldDeclId, docs: Option<Span>) {
        self.ast.fields[id].docs = docs;
    }

    /// Adds an enum variant declaration and returns its ID.
    pub fn push_variant(
        &mut self,
        name: Symbol,
        name_span: Span,
        fields: VariantFieldList,
        span: Span,
    ) -> VariantDeclId {
        self.ast.variants.push(VariantDecl {
            name,
            name_span,
            docs: None,
            fields,
            cfg: None,
            deprecated: None,
            span,
        })
    }

    /// Adds an enum variant with its rejected `cfg` condition.
    pub fn push_variant_with_cfg(
        &mut self,
        name: Symbol,
        name_span: Span,
        fields: VariantFieldList,
        span: Span,
        cfg: Option<CfgExprId>,
        deprecated: Option<seal_def::Deprecation>,
    ) -> VariantDeclId {
        self.ast.variants.push(VariantDecl {
            name,
            name_span,
            docs: None,
            fields,
            cfg,
            deprecated,
            span,
        })
    }

    /// Attaches outer doc comments to an enum variant.
    pub fn set_variant_docs(&mut self, id: VariantDeclId, docs: Option<Span>) {
        self.ast.variants[id].docs = docs;
    }

    /// Attaches inner doc comments to a module declaration.
    pub fn set_module_docs(&mut self, id: ItemId, docs: Option<Span>) {
        if let ItemKind::Module(declaration) = &mut self.ast.items[id].kind {
            declaration.docs = docs;
        }
    }

    /// Adds a positional field of an enum variant and returns its ID.
    pub fn push_tuple_field(&mut self, ty: TypeId, span: Span) -> TupleFieldDeclId {
        self.ast.tuple_fields.push(TupleFieldDecl { ty, span })
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
        mutable: bool,
        ty: Option<TypeId>,
        init: Option<ExprId>,
    ) -> StmtId {
        self.ast.stmts.push(Stmt {
            kind: StmtKind::Let(LetStmt {
                name,
                name_span,
                mutable,
                ty,
                init,
            }),
            span,
        })
    }

    /// Adds a block-scope constant declaration and returns its ID.
    pub fn push_const_stmt(&mut self, span: Span, declaration: ConstDecl) -> StmtId {
        self.ast.stmts.push(Stmt {
            kind: StmtKind::Const(declaration),
            span,
        })
    }

    /// Adds a block-scope static declaration and returns its ID.
    pub fn push_static_stmt(&mut self, span: Span, declaration: StaticDecl) -> StmtId {
        self.ast.stmts.push(Stmt {
            kind: StmtKind::Static(declaration),
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

    /// Adds a `while` statement and returns its ID.
    pub fn push_while(&mut self, span: Span, condition: ExprId, body: BlockId) -> StmtId {
        self.ast.stmts.push(Stmt {
            kind: StmtKind::While { condition, body },
            span,
        })
    }

    /// Adds a `while let` statement and returns its ID.
    pub fn push_while_let(
        &mut self,
        span: Span,
        pattern: PatternId,
        scrutinee: ExprId,
        body: BlockId,
    ) -> StmtId {
        self.ast.stmts.push(Stmt {
            kind: StmtKind::WhileLet {
                pattern,
                scrutinee,
                body,
            },
            span,
        })
    }

    /// Adds a `for` statement and returns its ID.
    pub fn push_for(
        &mut self,
        span: Span,
        pattern: crate::id::PatternId,
        source: crate::block::ForSource,
        body: BlockId,
    ) -> StmtId {
        self.ast.stmts.push(Stmt {
            kind: StmtKind::For {
                pattern,
                source,
                body,
            },
            span,
        })
    }

    /// Adds a `break` statement and returns its ID.
    pub fn push_break(&mut self, span: Span) -> StmtId {
        self.ast.stmts.push(Stmt {
            kind: StmtKind::Break,
            span,
        })
    }

    /// Adds a `continue` statement and returns its ID.
    pub fn push_continue(&mut self, span: Span) -> StmtId {
        self.ast.stmts.push(Stmt {
            kind: StmtKind::Continue,
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

    /// Adds an `if` expression and returns its ID.
    pub fn push_if(
        &mut self,
        span: Span,
        condition: ExprId,
        then_branch: BlockId,
        else_branch: Option<BlockId>,
    ) -> ExprId {
        self.ast.exprs.push(Expr {
            kind: ExprKind::If {
                condition,
                then_branch,
                else_branch,
            },
            span,
        })
    }

    /// Adds an `if let` expression and returns its ID.
    pub fn push_if_let(
        &mut self,
        span: Span,
        pattern: PatternId,
        scrutinee: ExprId,
        then_branch: BlockId,
        else_branch: Option<BlockId>,
    ) -> ExprId {
        self.ast.exprs.push(Expr {
            kind: ExprKind::IfLet {
                pattern,
                scrutinee,
                then_branch,
                else_branch,
            },
            span,
        })
    }

    /// Adds an `unsafe` block expression and returns its ID.
    pub fn push_unsafe(&mut self, span: Span, block: BlockId) -> ExprId {
        self.ast.exprs.push(Expr {
            kind: ExprKind::Unsafe(block),
            span,
        })
    }

    /// Adds an expression and returns its ID.
    pub fn push_expr(&mut self, kind: ExprKind, span: Span) -> ExprId {
        self.ast.exprs.push(Expr { kind, span })
    }

    /// Adds a pattern and returns its ID.
    pub fn push_pattern(&mut self, kind: crate::pattern::PatternKind, span: Span) -> PatternId {
        self.ast.patterns.push(Pattern { kind, span })
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

    /// Records the inner docs of a file that has no module declaration.
    pub fn set_root_docs(&mut self, docs: Option<Span>) {
        self.ast.module_docs = docs;
    }

    /// Attaches the program's items and returns the finished tree.
    pub fn finish(mut self, items: Vec<ItemId>) -> Ast {
        self.ast.program = Program { items };
        self.ast
    }
}
