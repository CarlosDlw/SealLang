//! Function bodies and trait default bodies.

use seal_ast::{BlockId as AstBlockId, ItemKind, ParamId, TypeId as AstTypeId};
use seal_def::DefId;
use seal_ty::TyId;

use super::Lowering;
use crate::body::{Body, BodyKind, ParamData};

/// The two signature markers that are copied onto a lowered body.
#[derive(Clone, Copy)]
struct BodyFlags {
    is_const: bool,
    is_unsafe: bool,
}

impl Lowering<'_> {
    pub(super) fn lower_bodies(&mut self) {
        let items = self.function_items.clone();
        for item_id in items {
            let Some(&function) = self.function_defs.get(&item_id) else {
                continue;
            };
            let (ret, params, body, is_const, is_unsafe) = {
                let item = self.ast.item(item_id);
                let ItemKind::Func(declaration) = &item.kind else {
                    continue;
                };
                (
                    declaration.ret,
                    declaration.params.clone(),
                    declaration.body,
                    declaration.is_const,
                    declaration.is_unsafe,
                )
            };

            let self_ty = self.impl_self_ty_of_function.get(&function).copied();
            self.lower_body(
                function,
                ret,
                &params,
                body,
                self_ty,
                BodyFlags {
                    is_const,
                    is_unsafe,
                },
            );
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
            let (ret, params, body, is_unsafe) = {
                let item = self.ast.item(item_id);
                let ItemKind::TraitMethod(declaration) = &item.kind else {
                    continue;
                };
                let Some(body) = declaration.body else {
                    continue;
                };
                (
                    declaration.ret,
                    declaration.params.clone(),
                    body,
                    declaration.is_unsafe,
                )
            };

            self.lower_body(
                method,
                ret,
                &params,
                body,
                None,
                BodyFlags {
                    is_const: false,
                    is_unsafe,
                },
            );
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
        flags: BodyFlags,
    ) {
        // Bodies are independent: a closure frame left behind by the enclosing
        // body must not reach the next one.
        let saved_closures = std::mem::take(&mut self.closure_env);
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
        self.closure_env = saved_closures;

        let body = self.hir.push_body(Body {
            owner,
            kind: BodyKind::Function,
            ret,
            is_const: flags.is_const,
            is_unsafe: flags.is_unsafe,
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
        self.define_param(function, name, name_span, span, ty)
    }
}
