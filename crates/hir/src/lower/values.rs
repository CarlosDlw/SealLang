//! Constant and static definitions and their initializer bodies.
//!
//! A constant or static initializer is an expression, not a block, but it is
//! still evaluated on its own: it gets a body with no parameters and the
//! declared type as its return type. The body is owned by the value definition,
//! never by the enclosing function, so a local static has one initializer even
//! when its function is instantiated several times.

use seal_ast::{ExprId as AstExprId, GenericParams, ItemId, ItemKind, TypeId as AstTypeId};
use seal_def::{DefData, DefId, DefKind};
use seal_error_codes as codes;
use seal_span::Span;
use seal_symbol::Symbol;

use super::{CaptureContext, Lowering};
use crate::BodyId;
use crate::body::{Block, Body, BodyKind};

impl Lowering<'_> {
    /// Registers one file-scope constant or static in the value namespace.
    pub(super) fn collect_value(
        &mut self,
        item_id: ItemId,
        kind: DefKind,
        name: Symbol,
        name_span: Span,
        mutable: bool,
        span: Span,
    ) {
        let definition = self.define_item(
            item_id,
            DefData::new(kind, name, name_span, span, None, mutable),
        );
        self.register_generics(definition, &GenericParams::empty(span), None, false);
        self.value_items.push((self.item_ref(item_id), definition));

        if let Some(first) = self.local_value(name) {
            self.duplicate(
                first,
                name_span,
                format!(
                    "the value `{}` is defined twice",
                    self.interner.resolve(name)
                ),
                codes::E2068,
            );
        } else {
            self.globals.insert((self.current_module, name), definition);
        }
    }

    /// Lowers the initializer of every file-scope constant and static.
    pub(super) fn lower_value_bodies(&mut self) {
        let items = self.value_items.clone();
        for (item, definition) in items {
            self.enter_file(item.file);
            let item_id = item.id;
            let (ty, init, kind) = match &self.ast.item(item_id).kind {
                ItemKind::Const(declaration) => (declaration.ty, declaration.init, BodyKind::Const),
                ItemKind::Static(declaration) => (
                    declaration.ty,
                    declaration.init,
                    BodyKind::Static {
                        mutable: declaration.mutable,
                    },
                ),
                _ => continue,
            };
            self.lower_value_body(definition, ty, init, kind, None);
        }
        self.function = None;
    }

    /// Defines a block-scope constant or static.
    ///
    /// The name is not inserted into the enclosing scope here: the initializer
    /// is lowered first, so a declaration is not visible in its own initializer.
    pub(super) fn define_local_value(
        &mut self,
        kind: DefKind,
        name: Symbol,
        name_span: Span,
        span: Span,
        mutable: bool,
    ) -> DefId {
        let parent = self.function;
        let definition = self.define(DefData::new(kind, name, name_span, span, parent, mutable));
        self.register_generics(definition, &GenericParams::empty(span), None, false);
        definition
    }

    /// Inserts a block-scope value into the innermost scope.
    ///
    /// A duplicate in the same block is `E2068`; shadowing an outer scope is
    /// allowed and needs no special case.
    pub(super) fn declare_local_value(&mut self, definition: DefId, name: Symbol, name_span: Span) {
        if let Some(first) = self.scopes.current(name) {
            let first_span = self.hir.definitions().get(first).name_span;
            self.error(
                codes::E2068,
                name_span,
                format!(
                    "the value `{}` is defined twice",
                    self.interner.resolve(name)
                ),
            )
            .secondary(first_span, "first defined here")
            .help("rename one of the declarations")
            .emit();
        } else {
            self.scopes.insert(name, definition);
        }
    }

    /// Lowers one constant or static initializer into its own body.
    ///
    /// `capture` is the barrier of the enclosing body for a local static. The
    /// initializer runs outside that frame, so resolution rejects names owned
    /// by the function or closures around it, their generic parameters, and
    /// `Self`. The closure environment stack is cleared for the same reason: a
    /// static initializer never captures.
    pub(super) fn lower_value_body(
        &mut self,
        owner: DefId,
        ty: AstTypeId,
        init: AstExprId,
        kind: BodyKind,
        capture: Option<CaptureContext>,
    ) -> BodyId {
        let saved_function = self.function.replace(owner);
        let saved_loop_depth = std::mem::replace(&mut self.loop_depth, 0);
        let saved_generics = std::mem::take(&mut self.generic_scopes);
        let saved_self = self.self_ty.take();
        let saved_capture = std::mem::replace(&mut self.capture, capture);
        let saved_closures = std::mem::take(&mut self.closure_env);

        let levels = self.push_generic_scope(owner);
        self.scopes.push();
        let ret = self.lower_type(ty);
        let value = self.lower_expr(init);
        let span = self.ast.expr(init).span;
        let root = self.hir.push_block(Block {
            statements: Vec::new(),
            tail: Some(value),
            span,
        });
        self.scopes.pop();
        self.pop_generic_scope(levels);

        self.closure_env = saved_closures;
        self.capture = saved_capture;
        self.self_ty = saved_self;
        self.generic_scopes = saved_generics;
        self.loop_depth = saved_loop_depth;
        self.function = saved_function;

        let body = self.hir.push_body(Body {
            owner,
            kind,
            ret: Some(ret),
            is_const: false,
            is_unsafe: false,
            params: Vec::new(),
            root,
        });
        self.hir.set_body_of(owner, body);
        body
    }
}
