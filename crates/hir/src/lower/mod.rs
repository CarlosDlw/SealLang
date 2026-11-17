//! AST lowering and name resolution.

use seal_ast::{Ast, BlockId as AstBlockId, ImplDecl, ItemId, ItemKind, ParamId};
use seal_def::{DefData, DefId, DefKind};
use seal_diagnostics::{DiagCtxt, ErrorBuilder};
use seal_error_codes as codes;
use seal_error_codes::ErrorCode;
use seal_hash::FxHashMap;
use seal_span::Span;
use seal_symbol::{Interner, Symbol};
use seal_ty::{RefKind, TyCtxt, TyKind};

use crate::body::{Body, ParamData};
use crate::res::Res;
use crate::{Block, Hir, HirBlockId, ReceiverKind};

mod expr;
mod scope;
mod stmt;
mod structs;
mod types;

pub(crate) use types::intern_primitives;

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
    types: FxHashMap<Symbol, DefId>,
    function_defs: FxHashMap<ItemId, DefId>,
    function_items: Vec<ItemId>,
    struct_items: Vec<(ItemId, DefId)>,
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
            types: FxHashMap::default(),
            function_defs: FxHashMap::default(),
            function_items: Vec::new(),
            struct_items: Vec::new(),
            function: None,
            loop_depth: 0,
        }
    }

    fn run(mut self) -> Hir {
        self.collect_items();
        self.lower_structs();
        self.validate_structs();
        self.lower_bodies();
        self.hir
    }

    /// Creates a definition for every item before any body is lowered, so calls
    /// and type annotations work regardless of declaration order.
    ///
    /// Collection runs in dependency order: structs first, then free functions,
    /// then impls. An impl needs its target struct to exist, and a body that
    /// calls `Point::new` needs the impl registry to be complete, so bodies are
    /// lowered only after all three passes.
    fn collect_items(&mut self) {
        let items: Vec<ItemId> = self.ast.program().items().to_vec();

        for &item_id in &items {
            let ItemKind::Struct(declaration) = &self.ast.item(item_id).kind else {
                continue;
            };
            let span = self.ast.item(item_id).span;
            self.collect_struct(item_id, declaration.name, declaration.name_span, span);
        }

        for &item_id in &items {
            let ItemKind::Func(declaration) = &self.ast.item(item_id).kind else {
                continue;
            };
            let span = self.ast.item(item_id).span;
            self.collect_function(item_id, declaration.name, declaration.name_span, span);
        }

        for &item_id in &items {
            let ItemKind::Impl(declaration) = &self.ast.item(item_id).kind else {
                continue;
            };
            let declaration = declaration.clone();
            self.collect_impl(&declaration);
        }
    }

    /// Registers every function in one impl block under its target type.
    fn collect_impl(&mut self, declaration: &ImplDecl) {
        let Some(&type_def) = self.types.get(&declaration.type_name) else {
            let name = self.interner.resolve(declaration.type_name);
            if self.globals.contains_key(&declaration.type_name) {
                self.error(
                    codes::E2015,
                    declaration.type_span,
                    format!("`{name}` is a value, not a type"),
                )
                .help("an impl block attaches methods to a struct")
                .emit();
            } else {
                self.error(
                    codes::E2015,
                    declaration.type_span,
                    format!("cannot find struct `{name}` for this impl"),
                )
                .help("declare the struct with `struct`, then implement it")
                .emit();
            }
            return;
        };

        for &item_id in &declaration.functions {
            self.collect_impl_function(item_id, type_def);
        }
    }

    fn collect_impl_function(&mut self, item_id: ItemId, type_def: DefId) {
        let ItemKind::Func(declaration) = &self.ast.item(item_id).kind else {
            return;
        };
        let (name, name_span) = (declaration.name, declaration.name_span);
        let span = self.ast.item(item_id).span;
        let definition = self.define(DefData {
            kind: DefKind::Function,
            name,
            name_span,
            span,
            parent: Some(type_def),
            mutable: false,
        });
        self.function_defs.insert(item_id, definition);
        self.function_items.push(item_id);

        if let Some(first) = self.hir.method(type_def, name) {
            let first_span = self.hir.definitions().get(first).name_span;
            self.error(
                codes::E2017,
                name_span,
                format!(
                    "the associated function `{}` is defined twice",
                    self.interner.resolve(name)
                ),
            )
            .secondary(first_span, "first defined here")
            .help("give each impl function a distinct name")
            .emit();
        } else {
            self.hir.insert_method(type_def, name, definition);
        }

        let params = declaration.params.clone();
        let Some(receiver_param) = params.first().copied() else {
            return;
        };
        if self.is_self_parameter(receiver_param) {
            let receiver_ty = self.ast.param(receiver_param).ty;
            match self.receiver_kind(receiver_ty, type_def) {
                Some(ReceiverKind::Value) => {
                    self.hir.insert_receiver(definition, ReceiverKind::Value);
                }
                Some(ReceiverKind::Shared) => {
                    self.hir.insert_receiver(definition, ReceiverKind::Shared);
                }
                Some(ReceiverKind::Exclusive) => {
                    self.hir
                        .insert_receiver(definition, ReceiverKind::Exclusive);
                }
                None => self.receiver_error(receiver_param, type_def),
            }
        } else {
            for &param in params.iter().skip(1) {
                if self.is_self_parameter(param) {
                    self.error(
                        codes::E2016,
                        self.ast.param(param).name_span,
                        "`self` must be the first parameter",
                    )
                    .help("move `self` to the first position, or rename the parameter")
                    .emit();
                }
            }
        }
    }

    fn is_self_parameter(&self, param: ParamId) -> bool {
        self.interner.resolve(self.ast.param(param).name) == "self"
    }

    fn receiver_kind(&mut self, ty: seal_ast::TypeId, type_def: DefId) -> Option<ReceiverKind> {
        let adt = self.hir.adt(type_def)?;
        let target = self.hir.ty_ctxt().adt_ty(adt);
        let receiver = self.lower_type(ty);
        match self.hir.ty_ctxt().kind(receiver) {
            TyKind::Adt(found) if found == adt => Some(ReceiverKind::Value),
            TyKind::Ref {
                kind: RefKind::Shared,
                pointee,
            } if pointee == target => Some(ReceiverKind::Shared),
            TyKind::Ref {
                kind: RefKind::Exclusive,
                pointee,
            } if pointee == target => Some(ReceiverKind::Exclusive),
            _ => None,
        }
    }

    fn receiver_error(&mut self, param: ParamId, type_def: DefId) {
        let name = self
            .interner
            .resolve(self.hir.definitions().get(type_def).name);
        let span = self.ast.param(param).span;
        self.error(
            codes::E2016,
            span,
            format!("this is not a valid receiver for `{name}`"),
        )
        .help(format!(
            "write `self: {name}`, `self: *{name}`, or `self: *mut {name}`"
        ))
        .emit();
    }

    fn collect_function(&mut self, item_id: ItemId, name: Symbol, name_span: Span, span: Span) {
        let definition = self.define(DefData {
            kind: DefKind::Function,
            name,
            name_span,
            span,
            parent: None,
            mutable: false,
        });
        self.function_defs.insert(item_id, definition);
        self.function_items.push(item_id);

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

    fn collect_struct(&mut self, item_id: ItemId, name: Symbol, name_span: Span, span: Span) {
        let definition = self.define(DefData {
            kind: DefKind::Struct,
            name,
            name_span,
            span,
            parent: None,
            mutable: false,
        });
        let adt = self.hir.ty_ctxt().push_adt(definition, name);
        self.hir.insert_struct(definition, adt);
        self.struct_items.push((item_id, definition));

        if let Some(first) = self.types.get(&name).copied() {
            let first_span = self.hir.definitions().get(first).name_span;
            self.error(
                codes::E2005,
                name_span,
                format!(
                    "the type `{}` is defined twice",
                    self.interner.resolve(name)
                ),
            )
            .secondary(first_span, "first defined here")
            .help("give each struct a distinct name")
            .emit();
        } else {
            self.types.insert(name, definition);
        }
    }

    fn lower_bodies(&mut self) {
        let items = self.function_items.clone();
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
        if self.types.contains_key(&name) {
            self.error(
                codes::E2010,
                span,
                format!("`{}` is a type, not a value", self.interner.resolve(name)),
            )
            .help("construct it with a struct literal, such as `Name { field: value }`")
            .emit();
            return None;
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
