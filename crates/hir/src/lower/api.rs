//! Public API validation.
//!
//! A `public` item is part of the API only when its module, and every ancestor,
//! is exposed. The API also has to be usable: a public signature that mentions a
//! non-public type cannot be called from another package, so it is rejected
//! rather than documented as reachable.

use seal_def::{DefId, DefKind, Visibility};
use seal_error_codes as codes;
use seal_span::Span;
use seal_ty::{TraitDef, TyId, TyKind};

use super::Lowering;

impl Lowering<'_> {
    /// Validates visibility and signature reachability, then records the API.
    pub(super) fn validate_api(&mut self) {
        let definitions: Vec<DefId> = self
            .hir
            .definitions()
            .iter_enumerated()
            .map(|(id, _)| id)
            .collect();
        for def in definitions {
            if self.hir.is_builtin_def(def) {
                continue;
            }
            let data = *self.hir.definitions().get(def);
            if data.parent.is_some() || data.kind == DefKind::Impl {
                continue;
            }
            if data.visibility != Visibility::Public {
                continue;
            }
            if !self.hir.module(data.module).exposed {
                self.error(
                    codes::E2080,
                    data.name_span,
                    format!(
                        "the public item `{}` is not reachable because its module is not exposed",
                        self.interner.resolve(data.name)
                    ),
                )
                .help("expose the module in the package API, or make the item `internal`")
                .emit();
                continue;
            }
            self.hir.api_mut().push_item(def);
            self.validate_item(def, data.kind, data.name_span, data.span);
        }

        let exports: Vec<_> = self
            .hir
            .module_exports()
            .map(|(owner, name, binding)| (owner, name, *binding))
            .collect();
        for (owner, name, binding) in exports {
            if binding.visibility != Visibility::Public || !self.hir.module(owner).exposed {
                continue;
            }
            let owner_path = self.hir.module(owner).path.clone();
            let owner_package = self.hir.module(owner).package;
            match binding.target {
                crate::module::ExportTarget::Module { module } => {
                    self.hir
                        .api_mut()
                        .push_module_alias(crate::module::ApiModuleAlias {
                            package: owner_package,
                            path: owner_path.child(name),
                            module,
                        });
                }
                crate::module::ExportTarget::Item {
                    module,
                    name: target_name,
                    namespace,
                } => {
                    let target = match namespace {
                        crate::module::Namespace::Value => self.globals.get(&(module, target_name)),
                        crate::module::Namespace::Type => self.types.get(&(module, target_name)),
                        crate::module::Namespace::Trait => self.traits.get(&(module, target_name)),
                    }
                    .copied();
                    if let Some(def) = target {
                        self.hir.api_mut().push_alias(crate::module::ApiAlias {
                            package: owner_package,
                            path: owner_path,
                            name,
                            def,
                        });
                    }
                }
            }
        }
    }

    fn validate_item(&mut self, def: DefId, kind: DefKind, name_span: Span, span: Span) {
        match kind {
            DefKind::Function | DefKind::ExternFunction => {
                self.validate_function(def, span);
            }
            DefKind::Const | DefKind::Static | DefKind::ExternStatic => {
                if let Some(body) = self.hir.body_of(def)
                    && let Some(ret) = body.ret
                {
                    self.require_api_type(ret, span, codes::E3101, "this public value");
                }
            }
            DefKind::Struct => self.validate_struct(def, name_span),
            DefKind::Enum => self.validate_enum(def, span),
            DefKind::Trait => self.validate_trait(def, span),
            DefKind::TypeAlias => self.validate_type_alias(def, name_span),
            DefKind::OpaqueType => {}
            _ => {}
        }
    }

    fn validate_type_alias(&mut self, def: DefId, name_span: Span) {
        let Some(target) = self.hir.type_alias_target(def) else {
            return;
        };
        self.require_api_type(target, name_span, codes::E3107, "this public type alias");
    }

    fn validate_function(&mut self, def: DefId, span: Span) {
        if let Some(body) = self.hir.body_of(def) {
            let params: Vec<TyId> = body.params.iter().map(|param| param.ty).collect();
            let ret = body.ret;
            for ty in params {
                self.require_api_type(ty, span, codes::E3100, "this public function");
            }
            if let Some(ret) = ret {
                self.require_api_type(ret, span, codes::E3100, "this public function");
            }
            return;
        }
        let external = self.hir.extern_function(def).cloned();
        if let Some(external) = external {
            for param in &external.params {
                self.require_api_type(param.ty, span, codes::E3100, "this public function");
            }
            if let Some(ret) = external.ret {
                self.require_api_type(ret, span, codes::E3100, "this public function");
            }
        }
    }

    fn validate_struct(&mut self, def: DefId, span: Span) {
        let Some(adt) = self.hir.adt(def) else {
            return;
        };
        let fields = self.hir.ty_ctxt().adt(adt).fields.clone();
        for field in &fields {
            let field_def = *self.hir.definitions().get(field.def);
            if field_def.visibility != Visibility::Public {
                continue;
            }
            self.hir.api_mut().push_item(field.def);
            self.require_api_type(
                field.ty,
                field_def.name_span,
                codes::E3104,
                "this public field",
            );
        }
        let _ = span;
    }

    fn validate_enum(&mut self, def: DefId, span: Span) {
        let Some(adt) = self.hir.adt(def) else {
            return;
        };
        let variants = self.hir.ty_ctxt().adt(adt).variants.clone();
        for variant in &variants {
            self.hir.api_mut().push_item(variant.def);
            for field in &variant.fields {
                self.hir.api_mut().push_item(field.def);
                self.require_api_type(
                    field.ty,
                    self.hir.definitions().get(field.def).name_span,
                    codes::E3105,
                    "this public variant",
                );
            }
        }
        let _ = span;
    }

    fn validate_trait(&mut self, def: DefId, span: Span) {
        let Some(declaration) = self.hir.trait_definition(def) else {
            return;
        };
        for supertrait in &declaration.supertraits {
            self.require_api_trait(
                supertrait.trait_def,
                span,
                codes::E3103,
                "this public trait",
            );
        }
        let TraitDef { methods, .. } = declaration;
        for method in &methods {
            for param in &method.signature.params {
                self.require_api_type(*param, span, codes::E3102, "this public trait method");
            }
            self.require_api_type(
                method.signature.ret,
                span,
                codes::E3102,
                "this public trait method",
            );
        }
    }

    /// Reports a type that another package could not name.
    fn require_api_type(
        &mut self,
        ty: TyId,
        span: Span,
        code: seal_error_codes::ErrorCode,
        what: &str,
    ) {
        let Some(def) = self.non_public_type(ty) else {
            return;
        };
        let data = *self.hir.definitions().get(def);
        self.error(
            code,
            span,
            format!(
                "{what} exposes the non-public type `{}`",
                self.interner.resolve(data.name)
            ),
        )
        .secondary(data.name_span, "declared here")
        .help("make the type public and expose its module, or reduce the declaration's visibility")
        .emit();
    }

    fn require_api_trait(
        &mut self,
        trait_def: DefId,
        span: Span,
        code: seal_error_codes::ErrorCode,
        what: &str,
    ) {
        if self.hir.is_builtin_def(trait_def) || self.hir.is_public_api(trait_def) {
            return;
        }
        let data = *self.hir.definitions().get(trait_def);
        self.error(
            code,
            span,
            format!(
                "{what} exposes the non-public trait `{}`",
                self.interner.resolve(data.name)
            ),
        )
        .secondary(data.name_span, "declared here")
        .help("make the trait public and expose its module, or reduce the declaration's visibility")
        .emit();
    }

    /// Returns one definition that makes `ty` unnameable outside the package.
    fn non_public_type(&self, ty: TyId) -> Option<DefId> {
        let tcx = self.hir.ty_ctxt();
        match tcx.kind(ty) {
            TyKind::Adt { adt, .. } => {
                let def = tcx.adt(adt).def;
                (self.hir.definitions().kind(def) != DefKind::Closure
                    && !self.hir.is_builtin_def(def)
                    && !self.hir.is_public_api(def))
                .then_some(def)
            }
            TyKind::Opaque { def, .. } => (!self.hir.is_public_api(def)).then_some(def),
            TyKind::Dyn { trait_def, .. } => (!self.hir.is_builtin_def(trait_def)
                && !self.hir.is_public_api(trait_def))
            .then_some(trait_def),
            TyKind::Ref { pointee, .. }
            | TyKind::RawPtr { pointee, .. }
            | TyKind::Slice { element: pointee }
            | TyKind::Array {
                element: pointee, ..
            } => self.non_public_type(pointee),
            TyKind::Tuple(tuple) => tcx
                .tuple_elements(tuple)
                .iter()
                .find_map(|element| self.non_public_type(*element)),
            TyKind::FnSig(id) => {
                let signature = tcx.fn_sig(id);
                signature
                    .params
                    .iter()
                    .find_map(|param| self.non_public_type(*param))
                    .or_else(|| self.non_public_type(signature.ret))
            }
            TyKind::FnPtr(id) => {
                let pointer = tcx.fn_ptr_def(id);
                pointer
                    .params
                    .iter()
                    .find_map(|param| self.non_public_type(*param))
                    .or_else(|| self.non_public_type(pointer.ret))
            }
            _ => None,
        }
    }
}
