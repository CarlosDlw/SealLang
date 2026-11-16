//! AST lowering and name resolution.

use seal_ast::{Ast, BlockId as AstBlockId, ItemId, ItemKind};
use seal_def::{DefData, DefId, DefKind};
use seal_diagnostics::{DiagCtxt, ErrorBuilder};
use seal_error_codes as codes;
use seal_error_codes::ErrorCode;
use seal_hash::FxHashMap;
use seal_span::Span;
use seal_symbol::{Interner, Symbol};
use seal_ty::TyCtxt;

use crate::body::{Body, ParamData};
use crate::res::Res;
use crate::{Block, Hir, HirBlockId};

mod expr;
mod scope;
mod stmt;
mod types;

pub(crate) fn lower_ast(ast: &Ast, dcx: &DiagCtxt, interner: &Interner) -> Hir {
    Lowering::new(ast, dcx, interner).run()
}

/// One lowering pass over a parsed file.
pub(crate) struct Lowering<'a> {
    pub(super) ast: &'a Ast,
    pub(super) dcx: &'a DiagCtxt,
    pub(super) interner: &'a Interner,
    pub(super) hir: Hir,
    scopes: scope::Scopes,
    globals: FxHashMap<Symbol, DefId>,
    function_defs: FxHashMap<ItemId, DefId>,
    function: Option<DefId>,
    loop_depth: usize,
}

impl<'a> Lowering<'a> {
    fn new(ast: &'a Ast, dcx: &'a DiagCtxt, interner: &'a Interner) -> Self {
        Self {
            ast,
            dcx,
            interner,
            hir: Hir::empty(TyCtxt::new()),
            scopes: scope::Scopes::default(),
            globals: FxHashMap::default(),
            function_defs: FxHashMap::default(),
            function: None,
            loop_depth: 0,
        }
    }

    fn run(mut self) -> Hir {
        self.collect_functions();
        self.lower_bodies();
        self.hir
    }

    /// Creates a definition for every function before any body is lowered, so
    /// calls work regardless of declaration order.
    fn collect_functions(&mut self) {
        let items: Vec<ItemId> = self.ast.program().items().to_vec();
        for item_id in items {
            let (name, name_span, span) = {
                let item = self.ast.item(item_id);
                let ItemKind::Func(declaration) = &item.kind else {
                    continue;
                };
                (declaration.name, declaration.name_span, item.span)
            };
            let definition = self.define(DefData {
                kind: DefKind::Function,
                name,
                name_span,
                span,
                parent: None,
                mutable: false,
            });
            self.function_defs.insert(item_id, definition);

            if let Some(first) = self.globals.get(&name).copied() {
                let first_span = self.hir.definitions().get(first).name_span;
                self.error(
                    codes::E2001,
                    name_span,
                    format!(
                        "the function `{}` is defined twice",
                        self.interner.resolve(name)
                    ),
                )
                .secondary(first_span, "first defined here")
                .help("give each function a distinct name")
                .emit();
            } else {
                self.globals.insert(name, definition);
            }
        }
    }

    fn lower_bodies(&mut self) {
        let items: Vec<ItemId> = self.ast.program().items().to_vec();
        for item_id in items {
            let Some(&function) = self.function_defs.get(&item_id) else {
                continue;
            };
            let (ret, params, body) = {
                let item = self.ast.item(item_id);
                let ItemKind::Func(declaration) = &item.kind else {
                    continue;
                };
                (
                    declaration.ret,
                    declaration.params.clone(),
                    declaration.body,
                )
            };

            self.function = Some(function);
            self.loop_depth = 0;
            self.scopes.push();
            let ret = ret.map(|ty| self.lower_type(ty));
            let params = params
                .iter()
                .map(|param| self.lower_param(*param, function))
                .collect();
            let root = self.lower_block(body);
            self.scopes.pop();

            let body = self.hir.push_body(Body {
                owner: function,
                ret,
                params,
                root,
            });
            self.hir.set_body_of(function, body);
        }
        self.function = None;
    }

    fn lower_param(&mut self, param: seal_ast::ParamId, function: DefId) -> ParamData {
        let (name, name_span, ty, span) = {
            let param = self.ast.param(param);
            (param.name, param.name_span, param.ty, param.span)
        };
        let ty = self.lower_type(ty);
        let definition = self.define(DefData {
            kind: DefKind::Param,
            name,
            name_span,
            span,
            parent: Some(function),
            mutable: false,
        });

        if let Some(first) = self.scopes.current(name) {
            let first_span = self.hir.definitions().get(first).name_span;
            self.error(
                codes::E2001,
                name_span,
                format!(
                    "the parameter `{}` is declared twice",
                    self.interner.resolve(name)
                ),
            )
            .secondary(first_span, "first declared here")
            .help("rename one of the parameters")
            .emit();
        } else {
            self.scopes.insert(name, definition);
        }

        ParamData {
            def: definition,
            ty,
        }
    }

    /// Resolves a name against the local scopes, then the function namespace.
    pub(super) fn resolve(&self, name: Symbol, span: Span) -> Option<Res> {
        if let Some(definition) = self.scopes.lookup(name) {
            return Some(Res::Local(definition));
        }
        if let Some(&definition) = self.globals.get(&name) {
            return Some(Res::Function(definition));
        }

        self.error(
            codes::E2002,
            span,
            format!(
                "cannot find `{}` in this scope",
                self.interner.resolve(name)
            ),
        )
        .help("declare it with `let` or define a function with `func`")
        .emit();
        None
    }

    /// Builds a diagnostic with this phase's codes.
    pub(super) fn error(
        &self,
        code: ErrorCode,
        span: Span,
        message: impl Into<String>,
    ) -> ErrorBuilder<'_> {
        self.dcx
            .error(message)
            .code(code.diag_code())
            .primary(span, "")
    }

    fn define(&mut self, data: DefData) -> DefId {
        self.hir.define(data)
    }
}

impl Lowering<'_> {
    /// Lowers a block, entering its scope.
    pub(super) fn lower_block(&mut self, block: AstBlockId) -> HirBlockId {
        self.scopes.push();

        let statement_count = self.ast.block(block).statements.len();
        let mut statements = Vec::with_capacity(statement_count);
        for index in 0..statement_count {
            let statement = self.ast.block(block).statements[index];
            statements.push(self.lower_stmt(statement));
        }
        let tail = self.ast.block(block).tail.map(|expr| self.lower_expr(expr));
        let span = self.ast.block(block).span;

        self.scopes.pop();
        self.hir.push_block(Block {
            statements,
            tail,
            span,
        })
    }
}
