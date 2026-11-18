//! Function bodies and trait default bodies.

use seal_ast::{BlockId as AstBlockId, ItemKind, ParamId, TypeId as AstTypeId};
use seal_def::{DefData, DefId, DefKind};
use seal_error_codes as codes;
use seal_ty::TyId;

use super::Lowering;
use crate::body::{Body, BodyKind, ParamData};

impl Lowering<'_> {
    pub(super) fn lower_bodies(&mut self) {
        let items = self.function_items.clone();
        for item_id in items {
            let Some(&function) = self.function_defs.get(&item_id) else {
                continue;
            };
            let (ret, params, body, is_const) = {
                let item = self.ast.item(item_id);
                let ItemKind::Func(declaration) = &item.kind else {
                    continue;
                };
                (
                    declaration.ret,
                    declaration.params.clone(),
                    declaration.body,
                    declaration.is_const,
                )
            };

            let self_ty = self.impl_self_ty_of_function.get(&function).copied();
            self.lower_body(function, ret, &params, body, self_ty, is_const);
        }
        self.function = None;
    }

    /// Lowers every trait method that declares a default body.
    ///
    /// A default body belongs to the trait method itself, not to an impl, so
    /// `Self` stays the trait's implicit parameter. The body is otherwise an
    /// ordinary function body and is checked and lowered like one.
    pub(super) fn lower_trait_bodies(&mut self) {
        let items = self.trait_method_items.clone();
        for (item_id, method) in items {
            let (ret, params, body) = {
                let item = self.ast.item(item_id);
                let ItemKind::TraitMethod(declaration) = &item.kind else {
                    continue;
                };
                let Some(body) = declaration.body else {
                    continue;
                };
                (declaration.ret, declaration.params.clone(), body)
            };

            self.lower_body(method, ret, &params, body, None, false);
        }
        self.function = None;
    }

    /// Lowers one body under its owner's generic scope.
    ///
    /// `self_ty` replaces `Self` for an impl member; trait methods and free
    /// functions pass `None`, which leaves `Self` as a generic parameter.
    fn lower_body(
        &mut self,
        owner: DefId,
        ret: Option<AstTypeId>,
        params: &[ParamId],
        body: AstBlockId,
        self_ty: Option<TyId>,
        is_const: bool,
    ) {
        self.function = Some(owner);
        self.loop_depth = 0;
        let generic_levels = self.push_generic_scope(owner);
        let saved_self = self.self_ty;
        self.self_ty = self_ty;
        self.scopes.push();
        let ret = ret.map(|ty| self.lower_type(ty));
        let params = params
            .iter()
            .map(|param| self.lower_param(*param, owner))
            .collect();
        let root = self.lower_block(body);
        self.scopes.pop();
        self.self_ty = saved_self;
        self.pop_generic_scope(generic_levels);

        let body = self.hir.push_body(Body {
            owner,
            kind: BodyKind::Function,
            ret,
            is_const,
            params,
            root,
        });
        self.hir.set_body_of(owner, body);
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
}
