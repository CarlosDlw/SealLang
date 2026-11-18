//! Item collection and duplicate checks.

use seal_ast::{
    BlockId as AstBlockId, GenericParams, ImplDecl, ItemId, ItemKind, ParamId, TypeId as AstTypeId,
    TypeKind, WhereClause,
};
use seal_def::{DefData, DefId, DefKind};
use seal_error_codes as codes;
use seal_hash::FxHashMap;
use seal_span::Span;
use seal_symbol::Symbol;
use seal_ty::{AdtKind, GenericSignature, RefKind, TraitRef, TyId, TyKind};

use super::Lowering;
use crate::generics::InherentImpl;
use crate::{Protocol, ReceiverKind};

/// One method signature declared by a trait.
///
/// The parameter and return types stay in the AST because they name `Self`,
/// which only has a meaning per implementation. Validating an impl re-lowers
/// them with the concrete type bound to [`Lowering::self_ty`].
#[derive(Clone, Debug)]
pub(super) struct TraitMethodSig {
    pub(super) def: DefId,
    pub(super) name: Symbol,
    pub(super) name_span: Span,
    pub(super) receiver: Option<ReceiverKind>,
    /// Whether the declaration had a `self` parameter, even if it was invalid.
    pub(super) has_self_parameter: bool,
    /// Parameter types after the receiver, in declaration order.
    pub(super) params: Vec<(AstTypeId, Span)>,
    /// The declared return type, or `None` for unit.
    pub(super) ret: Option<AstTypeId>,
    /// The default body, or `None` when every implementation must supply one.
    pub(super) body: Option<AstBlockId>,
}

/// One `impl Trait for Type` block waiting for signature validation.
pub(super) struct PendingTraitImpl {
    pub(super) trait_ref: TraitRef,
    pub(super) self_ty: TyId,
    pub(super) span: Span,
    /// The functions in source order, as `(definition, name)`.
    pub(super) functions: Vec<(DefId, Symbol)>,
}

/// One implementation function's signature after `Self` substitution.
struct ImplSignature {
    name: Symbol,
    name_span: Span,
    params: Vec<TyId>,
    ret: TyId,
}

/// Maps a semantic trait-method receiver to the HIR receiver form.
fn protocol_receiver(method: &seal_ty::TraitMethodDef) -> Option<ReceiverKind> {
    method.receiver.map(|receiver| match receiver.reference {
        None => ReceiverKind::Value,
        Some(RefKind::Shared) => ReceiverKind::Shared,
        Some(RefKind::Exclusive) => ReceiverKind::Exclusive,
    })
}

impl Lowering<'_> {
    /// Creates a definition for every item before any body is lowered, so calls
    /// and type annotations work regardless of declaration order.
    ///
    /// Collection runs in dependency order: structs, enums, traits, free
    /// functions, and finally impl blocks. An impl needs its target type and,
    /// when it names a trait, the trait declaration; a body that calls
    /// `Point::new` needs the impl registry to be complete, so bodies are
    /// lowered only after every pass.
    pub(super) fn collect_items(&mut self) {
        let items: Vec<ItemId> = self.ast.program().items().to_vec();

        for &item_id in &items {
            let ItemKind::Struct(declaration) = &self.ast.item(item_id).kind else {
                continue;
            };
            let span = self.ast.item(item_id).span;
            self.collect_struct(item_id, declaration.name, declaration.name_span, span);
        }

        for &item_id in &items {
            let ItemKind::Enum(declaration) = &self.ast.item(item_id).kind else {
                continue;
            };
            let span = self.ast.item(item_id).span;
            self.collect_enum(item_id, declaration.name, declaration.name_span, span);
        }

        for &item_id in &items {
            let ItemKind::Trait(declaration) = &self.ast.item(item_id).kind else {
                continue;
            };
            let span = self.ast.item(item_id).span;
            self.collect_trait(item_id, declaration.name, declaration.name_span, span);
        }

        for &item_id in &items {
            let ItemKind::Func(declaration) = &self.ast.item(item_id).kind else {
                continue;
            };
            let span = self.ast.item(item_id).span;
            self.collect_function(item_id, declaration.name, declaration.name_span, span);
        }

        for &item_id in &items {
            let (kind, name, name_span, mutable, span) = match &self.ast.item(item_id).kind {
                ItemKind::Const(declaration) => (
                    DefKind::Const,
                    declaration.name,
                    declaration.name_span,
                    false,
                    self.ast.item(item_id).span,
                ),
                ItemKind::Static(declaration) => (
                    DefKind::Static,
                    declaration.name,
                    declaration.name_span,
                    declaration.mutable,
                    self.ast.item(item_id).span,
                ),
                _ => continue,
            };
            self.collect_value(item_id, kind, name, name_span, mutable, span);
        }

        for &item_id in &items {
            let ItemKind::Impl(declaration) = &self.ast.item(item_id).kind else {
                continue;
            };
            let span = self.ast.item(item_id).span;
            let declaration = declaration.clone();
            if declaration.trait_ref.is_some() {
                self.collect_trait_impl(item_id, &declaration, span);
            } else {
                self.collect_inherent_impl(item_id, &declaration, span);
            }
        }
    }

    /// Registers a trait and its method signatures.
    fn collect_trait(&mut self, item_id: ItemId, name: Symbol, name_span: Span, span: Span) {
        if let Some(protocol) = Protocol::from_name(self.interner.resolve(name)) {
            self.error(
                codes::E2066,
                name_span,
                format!(
                    "the trait name `{}` is reserved for a compiler-provided protocol",
                    protocol.name()
                ),
            )
            .help(format!(
                "`{}` is provided by the compiler; implement it instead of declaring it",
                protocol.name()
            ))
            .emit();
            return;
        }

        let definition = self.define(DefData {
            kind: DefKind::Trait,
            name,
            name_span,
            span,
            parent: None,
            mutable: false,
        });
        let (generics, supertraits, where_clause, methods) = {
            let ItemKind::Trait(declaration) = &self.ast.item(item_id).kind else {
                return;
            };
            (
                declaration.generics.clone(),
                declaration.supertraits.clone(),
                declaration.where_clause.clone(),
                declaration.methods.clone(),
            )
        };
        self.register_generics(definition, &generics, None, true);
        self.queue_bounds(definition, &generics, &supertraits, &where_clause, None);
        self.trait_items.push((item_id, definition));

        let existing = self
            .types
            .get(&name)
            .copied()
            .or_else(|| self.traits.get(&name).copied());
        if let Some(first) = existing {
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
            .help("give each struct, enum, and trait a distinct name")
            .emit();
        } else {
            self.traits.insert(name, definition);
            self.hir.insert_trait_name(name, definition);
        }

        let mut order = Vec::with_capacity(methods.len());
        let mut seen: FxHashMap<Symbol, Span> = FxHashMap::default();
        for method in methods {
            self.collect_trait_method(definition, method, &mut seen, &mut order);
        }
        self.trait_method_order.insert(definition, order);
    }

    /// Registers one trait method signature, rejecting duplicate names.
    fn collect_trait_method(
        &mut self,
        trait_def: DefId,
        item_id: ItemId,
        seen: &mut FxHashMap<Symbol, Span>,
        order: &mut Vec<Symbol>,
    ) {
        let ItemKind::TraitMethod(declaration) = &self.ast.item(item_id).kind else {
            return;
        };
        let (name, name_span) = (declaration.name, declaration.name_span);
        let params = declaration.params.clone();
        let ret = declaration.ret;
        let body = declaration.body;
        let generics_decl = declaration.generics.clone();
        let where_clause = declaration.where_clause.clone();

        if let Some(first_span) = seen.get(&name).copied() {
            self.error(
                codes::E2026,
                name_span,
                format!(
                    "the trait method `{}` is declared twice",
                    self.interner.resolve(name)
                ),
            )
            .secondary(first_span, "first declared here")
            .help("give each method a distinct name")
            .emit();
            return;
        }
        seen.insert(name, name_span);

        let definition = self.define(DefData {
            kind: DefKind::TraitMethod,
            name,
            name_span,
            span: self.ast.item(item_id).span,
            parent: Some(trait_def),
            mutable: false,
        });
        self.register_generics(definition, &generics_decl, Some(trait_def), false);
        self.queue_bounds(definition, &generics_decl, &[], &where_clause, None);
        self.trait_method_items.push((item_id, definition));

        let has_self_parameter = params.iter().any(|param| self.is_self_parameter(*param));
        let (receiver, parameters) = self.trait_signature(trait_def, &params);
        self.trait_methods.insert(
            (trait_def, name),
            TraitMethodSig {
                def: definition,
                name,
                name_span,
                receiver,
                has_self_parameter,
                params: parameters,
                ret,
                body,
            },
        );
        order.push(name);
    }

    /// Splits a trait method's parameters into its receiver and the rest.
    ///
    /// A trait receiver is always written against `Self`, because the concrete
    /// type is supplied by each implementation.
    fn trait_signature(
        &mut self,
        trait_def: DefId,
        params: &[ParamId],
    ) -> (Option<ReceiverKind>, Vec<(AstTypeId, Span)>) {
        let trait_name = self
            .interner
            .resolve(self.hir.definitions().name(trait_def))
            .to_owned();
        let mut receiver = None;
        let mut remaining = Vec::new();
        for (index, &param) in params.iter().enumerate() {
            if !self.is_self_parameter(param) {
                let data = self.ast.param(param);
                remaining.push((data.ty, data.span));
                continue;
            }
            if index != 0 {
                let span = self.ast.param(param).name_span;
                self.error(codes::E2016, span, "`self` must be the first parameter")
                    .help("move `self` to the first position, or rename the parameter")
                    .emit();
                continue;
            }
            let receiver_ty = self.ast.param(param).ty;
            match self.trait_receiver_kind(receiver_ty) {
                Some(kind) => receiver = Some(kind),
                None => {
                    let span = self.ast.param(param).span;
                    self.error(
                        codes::E2016,
                        span,
                        format!("this is not a valid receiver for the trait `{trait_name}`"),
                    )
                    .help("write `self: Self`, `self: *Self`, or `self: *mut Self`")
                    .emit();
                }
            }
        }
        (receiver, remaining)
    }

    fn trait_receiver_kind(&self, ty: AstTypeId) -> Option<ReceiverKind> {
        match &self.ast.ty(ty).kind {
            TypeKind::Path(path) if path.name == self.self_symbol => Some(ReceiverKind::Value),
            TypeKind::Pointer { pointee, mutable } => {
                let TypeKind::Path(path) = &self.ast.ty(*pointee).kind else {
                    return None;
                };
                if path.name != self.self_symbol {
                    return None;
                }
                Some(if *mutable {
                    ReceiverKind::Exclusive
                } else {
                    ReceiverKind::Shared
                })
            }
            _ => None,
        }
    }

    /// Creates the definition and generic parameter list for one impl block.
    fn begin_impl(&mut self, declaration: &ImplDecl, span: Span) -> DefId {
        let definition = self.define(DefData {
            kind: DefKind::Impl,
            name: self.self_symbol,
            name_span: span,
            span,
            parent: None,
            mutable: false,
        });
        self.register_generics(definition, &declaration.generics, None, false);
        definition
    }

    /// Lowers the self type of an impl block with the impl parameters in scope.
    fn lower_impl_self_type(&mut self, declaration: &ImplDecl, impl_def: DefId) -> TyId {
        let levels = self.push_generic_scope(impl_def);
        if matches!(self.ast.ty(declaration.self_ty).kind, TypeKind::Dyn(_)) {
            let span = self.ast.ty(declaration.self_ty).span;
            self.error(
                codes::E2056,
                span,
                "a trait object cannot be an impl target",
            )
            .help("implement the trait for the concrete type instead")
            .emit();
            self.pop_generic_scope(levels);
            return self.hir.ty_ctxt().error_ty();
        }
        let path = match &self.ast.ty(declaration.self_ty).kind {
            TypeKind::Path(path) => Some((path.name, path.name_span)),
            _ => None,
        };
        if let Some((name, name_span)) = path {
            if !self.types.contains_key(&name)
                && !self.traits.contains_key(&name)
                && self.generic_scope_param(name).is_none()
            {
                let display = self.interner.resolve(name);
                let message = if self.globals.contains_key(&name) {
                    format!("`{display}` is a value, not a type")
                } else {
                    format!("cannot find type `{display}` for this impl")
                };
                self.error(codes::E2015, name_span, message)
                    .help("declare it with `struct` or `enum`, then implement it")
                    .emit();
                self.pop_generic_scope(levels);
                return self.hir.ty_ctxt().error_ty();
            }
        }
        let saved = self.self_ty.take();
        let ty = self.lower_type(declaration.self_ty);
        self.self_ty = saved;
        self.pop_generic_scope(levels);
        ty
    }

    /// Returns the nominal head of an impl target, when it has one.
    fn nominal_impl_target(&self, self_ty: TyId) -> Option<DefId> {
        match self.hir.ty_ctxt().kind(self_ty) {
            TyKind::Adt { adt, .. } => Some(self.hir.ty_ctxt().adt(adt).def),
            _ => None,
        }
    }

    /// Registers every function in one inherent impl block under its type.
    fn collect_inherent_impl(&mut self, item_id: ItemId, declaration: &ImplDecl, span: Span) {
        let impl_def = self.begin_impl(declaration, span);
        self.impl_items.push((item_id, impl_def));
        let self_ty = self.lower_impl_self_type(declaration, impl_def);
        self.queue_bounds(
            impl_def,
            &declaration.generics,
            &[],
            &declaration.where_clause,
            Some(self_ty),
        );

        let nominal = self.nominal_impl_target(self_ty);
        if nominal.is_none() && !matches!(self.hir.ty_ctxt().kind(self_ty), TyKind::Error) {
            let display = self.display_ty(self_ty);
            let ty_span = self.ast.ty(declaration.self_ty).span;
            self.error(
                codes::E2056,
                ty_span,
                format!("the inherent impl target `{display}` is not a nominal type"),
            )
            .help("inherent impls attach methods to a struct or enum declared in this file")
            .emit();
        }
        if !self.hir.ty_ctxt().is_sized(self_ty)
            && !matches!(self.hir.ty_ctxt().kind(self_ty), TyKind::Error)
        {
            let display = self.display_ty(self_ty);
            let ty_span = self.ast.ty(declaration.self_ty).span;
            self.error(
                codes::E2056,
                ty_span,
                format!("the impl target `{display}` has no known size"),
            )
            .help("implement the trait for a sized type instead")
            .emit();
        }

        self.impl_self_tys.insert(impl_def, self_ty);
        let mut methods = Vec::new();
        for &item_id in &declaration.functions {
            if let Some((name, definition)) =
                self.collect_inherent_function(item_id, impl_def, nominal, self_ty)
            {
                methods.push((name, definition));
            }
        }
        if let Some(generics) = self.hir.generics(impl_def) {
            self.hir.insert_inherent_impl(InherentImpl {
                def: impl_def,
                self_ty,
                generics,
                methods,
            });
        }
    }

    /// Registers every function in one trait impl block.
    fn collect_trait_impl(&mut self, item_id: ItemId, declaration: &ImplDecl, span: Span) {
        let impl_def = self.begin_impl(declaration, span);
        self.impl_items.push((item_id, impl_def));
        let self_ty = self.lower_impl_self_type(declaration, impl_def);
        self.queue_bounds(
            impl_def,
            &declaration.generics,
            &[],
            &declaration.where_clause,
            Some(self_ty),
        );
        self.impl_self_tys.insert(impl_def, self_ty);

        let levels = self.push_generic_scope(impl_def);
        let saved = self.self_ty.replace(self_ty);
        let trait_ref = declaration
            .trait_ref
            .as_ref()
            .and_then(|path| self.lower_impl_trait_ref(path, self_ty));
        self.self_ty = saved;
        self.pop_generic_scope(levels);

        if let Some(trait_ref) = trait_ref
            && let Some(protocol) = self.hir.protocol_of_trait(trait_ref.trait_def)
            && crate::protocols::is_builtin_sequence(self.hir.ty_ctxt(), self_ty)
        {
            let display = self.display_ty(self_ty);
            let ty_span = self.ast.ty(declaration.self_ty).span;
            self.error(
                codes::E2067,
                ty_span,
                format!(
                    "the protocol `{}` cannot be implemented for the compiler-owned type `{display}`",
                    protocol.name()
                ),
            )
            .help("wrap the array or slice in a struct and implement the protocol for the wrapper")
            .emit();
            return;
        }

        if !self.hir.ty_ctxt().is_sized(self_ty)
            && !matches!(self.hir.ty_ctxt().kind(self_ty), TyKind::Error)
        {
            let display = self.display_ty(self_ty);
            let ty_span = self.ast.ty(declaration.self_ty).span;
            self.error(
                codes::E2056,
                ty_span,
                format!("the impl target `{display}` has no known size"),
            )
            .help("implement the trait for a sized type instead")
            .emit();
        }

        let nominal = self.nominal_impl_target(self_ty);

        if let Some(trait_ref) = trait_ref {
            let trait_def = trait_ref.trait_def;
            let concrete = !self.hir.ty_ctxt().contains_param(self_ty)
                && self
                    .hir
                    .ty_ctxt()
                    .substs_args(trait_ref.args)
                    .into_iter()
                    .all(|argument| !self.hir.ty_ctxt().contains_param(argument));
            let implementations = self.hir.trait_impls();
            if concrete
                && let Some(first) = implementations
                    .iter()
                    .find(|implementation| implementation.trait_ref == trait_ref)
            {
                let first_span = self.hir.definitions().get(first.def).span;
                self.error(
                    codes::E2021,
                    span,
                    format!(
                        "the trait `{}` is already implemented for `{}`",
                        self.interner
                            .resolve(self.hir.definitions().name(trait_def)),
                        self.display_ty(self_ty)
                    ),
                )
                .secondary(first_span, "first implemented here")
                .help("remove this block, or merge its functions into the first one")
                .emit();
            }
        }

        let mut functions: Vec<(Symbol, DefId)> = Vec::new();
        let mut pending = Vec::with_capacity(declaration.functions.len());
        for &function_item in &declaration.functions {
            let Some((name, definition)) =
                self.define_impl_function(function_item, impl_def, nominal, self_ty)
            else {
                continue;
            };
            if let Some((_, first)) = functions.iter().find(|(existing, _)| *existing == name) {
                let first_span = self.hir.definitions().get(*first).name_span;
                let name_span = self.hir.definitions().get(definition).name_span;
                self.error(
                    codes::E2017,
                    name_span,
                    format!(
                        "the associated function `{}` is defined twice",
                        self.interner.resolve(name)
                    ),
                )
                .secondary(first_span, "first defined here")
                .help("give each function a distinct name")
                .emit();
            } else {
                functions.push((name, definition));
            }
            pending.push((definition, name));
        }

        let Some(trait_ref) = trait_ref else {
            return;
        };
        if let Some(type_def) = nominal {
            let map: FxHashMap<Symbol, DefId> = functions.iter().copied().collect();
            self.hir.insert_trait_impl(trait_ref, type_def, map);
        }
        if let Some(generics) = self.hir.generics(impl_def) {
            self.hir
                .ty_ctxt_mut()
                .push_trait_impl(seal_ty::TraitImplDef::new(
                    impl_def, trait_ref, generics, functions,
                ));
        }
        self.pending_trait_impls.push(PendingTraitImpl {
            trait_ref,
            self_ty,
            span,
            functions: pending,
        });
    }

    /// Defines one function inside an impl block and records its receiver.
    ///
    /// The caller decides which registry the function belongs to: an inherent
    /// function enters the per-type method map, while a trait function is bound
    /// to the trait implementation.
    fn define_impl_function(
        &mut self,
        item_id: ItemId,
        impl_def: DefId,
        type_def: Option<DefId>,
        self_ty: TyId,
    ) -> Option<(Symbol, DefId)> {
        let ItemKind::Func(declaration) = &self.ast.item(item_id).kind else {
            return None;
        };
        let (name, name_span) = (declaration.name, declaration.name_span);
        let span = self.ast.item(item_id).span;
        let params = declaration.params.clone();
        let generics = declaration.generics.clone();
        let where_clause = declaration.where_clause.clone();

        let definition = self.define(DefData {
            kind: DefKind::Function,
            name,
            name_span,
            span,
            parent: Some(type_def.unwrap_or(impl_def)),
            mutable: false,
        });
        self.function_defs.insert(item_id, definition);
        self.function_item_of.insert(definition, item_id);
        self.function_items.push(item_id);
        self.impl_self_ty_of_function.insert(definition, self_ty);
        self.register_generics(definition, &generics, Some(impl_def), false);
        self.queue_bounds(definition, &generics, &[], &where_clause, Some(self_ty));

        let Some(receiver_param) = params.first().copied() else {
            return Some((name, definition));
        };
        if self.is_self_parameter(receiver_param) {
            let levels = self.push_generic_scope(definition);
            let saved = self.self_ty.replace(self_ty);
            let receiver_ty = self.ast.param(receiver_param).ty;
            let receiver = self.receiver_kind(receiver_ty, self_ty);
            self.self_ty = saved;
            self.pop_generic_scope(levels);
            match receiver {
                Some(kind) => self.hir.insert_receiver(definition, kind),
                None => self.receiver_error(receiver_param, self_ty),
            }
        } else {
            for &param in params.iter().skip(1) {
                if self.is_self_parameter(param) {
                    let span = self.ast.param(param).name_span;
                    self.error(codes::E2016, span, "`self` must be the first parameter")
                        .help("move `self` to the first position, or rename the parameter")
                        .emit();
                }
            }
        }
        Some((name, definition))
    }

    /// Returns a previously registered inherent function with the same name and
    /// an impl self type that can equal `self_ty`.
    ///
    /// Two impl blocks may declare the same name for disjoint instances, such
    /// as `impl Pair<i32>` and `impl Pair<bool>`. Only overlapping blocks would
    /// make a call ambiguous, so only they are rejected.
    fn overlapping_inherent(&self, type_def: DefId, name: Symbol, self_ty: TyId) -> Option<DefId> {
        self.hir
            .methods(type_def, name)
            .iter()
            .copied()
            .find(|function| {
                self.impl_self_ty_of_function
                    .get(function)
                    .is_some_and(|existing| {
                        seal_ty::unify_templates(self.hir.ty_ctxt(), *existing, self_ty).is_some()
                    })
            })
    }

    /// Registers one inherent impl function under its type.
    fn collect_inherent_function(
        &mut self,
        item_id: ItemId,
        impl_def: DefId,
        type_def: Option<DefId>,
        self_ty: TyId,
    ) -> Option<(Symbol, DefId)> {
        let (name, definition) = self.define_impl_function(item_id, impl_def, type_def, self_ty)?;
        let Some(type_def) = type_def else {
            return Some((name, definition));
        };
        if let Some(first) = self.overlapping_inherent(type_def, name, self_ty) {
            let first_span = self.hir.definitions().get(first).name_span;
            let name_span = self.hir.definitions().get(definition).name_span;
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
        Some((name, definition))
    }

    /// Validates every trait implementation against its declaration.
    ///
    /// A trait is a complete contract: each declared method needs one function
    /// with a matching receiver, parameter types, and return type, and the block
    /// may not define anything else. `Self` in the declaration is replaced with
    /// the implementing struct before each comparison.
    pub(super) fn validate_trait_impls(&mut self) {
        let pending = std::mem::take(&mut self.pending_trait_impls);
        for implementation in pending {
            let trait_def = implementation.trait_ref.trait_def;
            let Some(declaration) = self.hir.trait_definition(trait_def) else {
                continue;
            };
            let protocol = self.hir.protocol_of_trait(trait_def);
            let order = if protocol.is_some() {
                declaration
                    .methods
                    .iter()
                    .map(|method| method.name)
                    .collect()
            } else {
                let Some(order) = self.trait_method_order.get(&trait_def).cloned() else {
                    continue;
                };
                order
            };
            let type_name = self.display_ty(implementation.self_ty);
            let trait_name = self
                .interner
                .resolve(self.hir.definitions().name(trait_def))
                .to_owned();

            let mut implemented: FxHashMap<Symbol, DefId> = FxHashMap::default();
            for &(function, name) in &implementation.functions {
                implemented.entry(name).or_insert(function);
            }

            for name in &order {
                let Some(method) = declaration.method(*name) else {
                    continue;
                };
                let has_default = if protocol.is_some() {
                    false
                } else {
                    self.trait_methods
                        .get(&(trait_def, *name))
                        .is_some_and(|raw| raw.body.is_some())
                };
                match implemented.get(name) {
                    None if has_default => {}
                    None => {
                        let help = if protocol.is_some() {
                            format!(
                                "add `func {}(...)` to this impl block; compiler-provided protocols have no default body",
                                self.interner.resolve(*name)
                            )
                        } else {
                            format!(
                                "add `func {}(...)` to this impl block, or give the trait method a default body",
                                self.interner.resolve(*name)
                            )
                        };
                        self.error(
                            codes::E2022,
                            implementation.span,
                            format!(
                                "the trait `{trait_name}` requires `{}`, which `{type_name}` does not implement",
                                self.interner.resolve(*name)
                            ),
                        )
                        .help(help)
                        .emit();
                    }
                    Some(&function) => {
                        if protocol.is_some() {
                            self.check_protocol_method(
                                method,
                                function,
                                implementation.trait_ref,
                                (&trait_name, &type_name),
                            );
                        } else {
                            let Some(raw) = self.trait_methods.get(&(trait_def, *name)).cloned()
                            else {
                                continue;
                            };
                            self.check_trait_method(
                                &raw,
                                method,
                                function,
                                implementation.trait_ref,
                                (&trait_name, &type_name),
                            );
                        }
                    }
                }
            }

            for &(function, name) in &implementation.functions {
                if order.contains(&name) {
                    continue;
                }
                let name_span = self.hir.definitions().get(function).name_span;
                self.error(
                    codes::E2023,
                    name_span,
                    format!(
                        "the trait `{trait_name}` does not declare `{}`",
                        self.interner.resolve(name)
                    ),
                )
                .help("move it into an inherent `impl` block, or declare it in the trait")
                .emit();
            }
        }
    }

    /// Lowers an implementation function's parameters and return type with the
    /// impl's `Self` substituted.
    fn lowered_impl_signature(&mut self, function: DefId, self_ty: TyId) -> Option<ImplSignature> {
        let item_id = *self.function_item_of.get(&function)?;
        let (name, params, ret, name_span) = {
            let ItemKind::Func(declaration) = &self.ast.item(item_id).kind else {
                return None;
            };
            (
                declaration.name,
                declaration.params.clone(),
                declaration.ret,
                declaration.name_span,
            )
        };

        let levels = self.push_generic_scope(function);
        let saved = self.self_ty.replace(self_ty);
        let mut implemented = Vec::with_capacity(params.len());
        for &param in &params {
            if self.is_self_parameter(param) {
                continue;
            }
            let data = self.ast.param(param);
            implemented.push(self.lower_type(data.ty));
        }
        let found_ret = match ret {
            Some(ty) => self.lower_type(ty),
            None => self.hir.ty_ctxt().unit_ty(),
        };
        self.self_ty = saved;
        self.pop_generic_scope(levels);

        Some(ImplSignature {
            name,
            name_span,
            params: implemented,
            ret: found_ret,
        })
    }

    /// Compares one implementation function with its source trait method.
    fn check_trait_method(
        &mut self,
        raw: &TraitMethodSig,
        declared: &seal_ty::TraitMethodDef,
        function: DefId,
        trait_ref: TraitRef,
        names: (&str, &str),
    ) {
        let (trait_name, type_name) = names;
        let Some(implemented) = self.lowered_impl_signature(function, trait_ref.self_ty) else {
            return;
        };

        let receiver = self.hir.receiver(function);
        let mut difference = if raw.receiver != receiver {
            Some("the receiver does not match the trait declaration".to_owned())
        } else if declared.signature.params.len() != implemented.params.len() {
            Some(format!(
                "the trait declares {} parameter(s), but this function has {}",
                declared.signature.params.len(),
                implemented.params.len()
            ))
        } else {
            match self.instantiate_trait_signature(declared, trait_ref, function) {
                Err(difference) => Some(difference),
                Ok(expected) => self.compare_parameter_types(&expected.params, &implemented.params),
            }
        };
        if difference.is_none() {
            if let Ok(expected) = self.instantiate_trait_signature(declared, trait_ref, function) {
                difference = self.compare_return_type(expected.ret, implemented.ret);
            }
        }

        let Some(difference) = difference else {
            return;
        };
        self.error(
            codes::E2024,
            implemented.name_span,
            format!(
                "`{}` does not match its declaration in the trait `{trait_name}`: {difference}",
                self.interner.resolve(implemented.name)
            ),
        )
        .secondary(raw.name_span, "declared here")
        .help(format!(
            "align the receiver, parameters, and return type; `{type_name}` plays the role of `Self`"
        ))
        .emit();
    }

    /// Compares one implementation function with a compiler-provided protocol
    /// method. Built-in declarations have no source span, so the primary
    /// diagnostic points at the implementation and never at a synthetic span.
    fn check_protocol_method(
        &mut self,
        method: &seal_ty::TraitMethodDef,
        function: DefId,
        trait_ref: TraitRef,
        names: (&str, &str),
    ) {
        let (trait_name, type_name) = names;
        let Some(implemented) = self.lowered_impl_signature(function, trait_ref.self_ty) else {
            return;
        };
        let receiver = self.hir.receiver(function);
        let expected_receiver = protocol_receiver(method);
        let mut difference = if expected_receiver != receiver {
            Some("the receiver does not match the compiler-provided declaration".to_owned())
        } else if method.signature.params.len() != implemented.params.len() {
            Some(format!(
                "the compiler provides {} parameter(s), but this function has {}",
                method.signature.params.len(),
                implemented.params.len()
            ))
        } else {
            match self.instantiate_trait_signature(method, trait_ref, function) {
                Err(difference) => Some(difference),
                Ok(expected) => self.compare_parameter_types(&expected.params, &implemented.params),
            }
        };
        if difference.is_none() {
            if let Ok(expected) = self.instantiate_trait_signature(method, trait_ref, function) {
                difference = self.compare_return_type(expected.ret, implemented.ret);
            }
        }

        let Some(difference) = difference else {
            return;
        };
        self.error(
            codes::E2024,
            implemented.name_span,
            format!(
                "`{}` does not match its compiler-provided declaration in the trait `{trait_name}`: {difference}",
                self.interner.resolve(implemented.name)
            ),
        )
        .help(format!(
            "align the receiver, parameters, and return type; `{type_name}` plays the role of `Self`"
        ))
        .emit();
    }

    /// Replaces the trait method's `Self`, trait, and method parameters with the
    /// implementation's header and the function's own parameters.
    ///
    /// The impl header already lowered its trait arguments, so `impl<A, B>
    /// Trait<B> for Pair<A, B>` maps the trait's argument to `B` regardless of
    /// declaration order. The method's own parameters map to the function's own
    /// parameters positionally, which is the only order they have.
    fn instantiate_trait_signature(
        &self,
        method: &seal_ty::TraitMethodDef,
        trait_ref: TraitRef,
        function: DefId,
    ) -> Result<GenericSignature, String> {
        let parameters = self.hir.ty_ctxt().flattened_generics(method.def);
        let trait_generics = self
            .hir
            .generics(trait_ref.trait_def)
            .map(|generics| generics.params.len())
            .unwrap_or(1);
        let method_params = self
            .hir
            .generics(function)
            .map(|generics| generics.params)
            .unwrap_or_default();
        if method_params.len() != method.generics.params.len() {
            return Err(format!(
                "the implementation declares {} generic parameter(s), but the trait declares {}",
                method_params.len(),
                method.generics.params.len()
            ));
        }
        let mut args = Vec::with_capacity(parameters.len());
        args.push(trait_ref.self_ty);
        args.extend(self.hir.ty_ctxt().substs_args(trait_ref.args));
        if args.len() != trait_generics {
            return Err(format!(
                "the implementation names {} trait argument(s), but the trait declares {}",
                args.len().saturating_sub(1),
                trait_generics.saturating_sub(1)
            ));
        }
        args.extend(
            method_params
                .iter()
                .map(|parameter| self.hir.ty_ctxt().param_ty(parameter.param)),
        );
        if args.len() != parameters.len() {
            return Err(format!(
                "the trait method has {} generic parameter(s), but this implementation provides {}",
                parameters.len().saturating_sub(trait_generics),
                method_params.len()
            ));
        }
        let substs = self.hir.ty_ctxt().substs(&args);
        let params = method
            .signature
            .params
            .iter()
            .map(|ty| self.hir.ty_ctxt().substitute(*ty, substs))
            .collect();
        let ret = self.hir.ty_ctxt().substitute(method.signature.ret, substs);
        Ok(GenericSignature::new(params, ret))
    }

    fn compare_parameter_types(&self, expected: &[TyId], found: &[TyId]) -> Option<String> {
        for (index, (expected, found)) in expected.iter().zip(found).enumerate() {
            if !self.trait_types_compatible(*expected, *found) {
                return Some(format!(
                    "parameter {} has type `{}`, but the trait declares `{}`",
                    index + 1,
                    self.display_ty(*found),
                    self.display_ty(*expected)
                ));
            }
        }
        None
    }

    fn compare_return_type(&self, expected: TyId, found: TyId) -> Option<String> {
        if self.trait_types_compatible(expected, found) {
            None
        } else {
            Some(format!(
                "the return type is `{}`, but the trait declares `{}`",
                self.display_ty(found),
                self.display_ty(expected)
            ))
        }
    }

    fn trait_types_compatible(&self, expected: TyId, found: TyId) -> bool {
        expected == found
            || matches!(self.hir.ty_ctxt().kind(expected), TyKind::Error)
            || matches!(self.hir.ty_ctxt().kind(found), TyKind::Error)
    }

    pub(super) fn display_ty(&self, ty: TyId) -> String {
        self.hir
            .ty_ctxt()
            .display_with(ty, self.interner)
            .to_string()
    }

    fn is_self_parameter(&self, param: ParamId) -> bool {
        self.interner.resolve(self.ast.param(param).name) == "self"
    }

    fn receiver_kind(&mut self, ty: seal_ast::TypeId, self_ty: TyId) -> Option<ReceiverKind> {
        let receiver = self.lower_type(ty);
        if receiver == self_ty {
            return Some(ReceiverKind::Value);
        }
        match self.hir.ty_ctxt().kind(receiver) {
            TyKind::Ref {
                kind: RefKind::Shared,
                pointee,
            } if pointee == self_ty => Some(ReceiverKind::Shared),
            TyKind::Ref {
                kind: RefKind::Exclusive,
                pointee,
            } if pointee == self_ty => Some(ReceiverKind::Exclusive),
            _ => None,
        }
    }

    fn receiver_error(&mut self, param: ParamId, self_ty: TyId) {
        let name = self.display_ty(self_ty);
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
        let (generics, where_clause) = match &self.ast.item(item_id).kind {
            ItemKind::Func(declaration) => (
                declaration.generics.clone(),
                declaration.where_clause.clone(),
            ),
            _ => (GenericParams::empty(span), WhereClause::empty(span)),
        };
        self.register_generics(definition, &generics, None, false);
        self.queue_bounds(definition, &generics, &[], &where_clause, None);

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
        let adt = self
            .hir
            .ty_ctxt()
            .push_adt(definition, name, AdtKind::Struct);
        self.hir.insert_adt(definition, adt);
        self.struct_items.push((item_id, definition));
        if let ItemKind::Struct(declaration) = &self.ast.item(item_id).kind {
            self.register_generics(definition, &declaration.generics, None, false);
            let count = self.own_generic_count(definition) as u32;
            self.hir.ty_ctxt().set_generic_count(adt, count);
            self.queue_bounds(
                definition,
                &declaration.generics,
                &[],
                &declaration.where_clause,
                None,
            );
        }

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
            .help("give each struct, enum, and trait a distinct name")
            .emit();
        } else {
            self.types.insert(name, definition);
        }
    }

    fn collect_enum(&mut self, item_id: ItemId, name: Symbol, name_span: Span, span: Span) {
        let definition = self.define(DefData {
            kind: DefKind::Enum,
            name,
            name_span,
            span,
            parent: None,
            mutable: false,
        });
        let adt = self.hir.ty_ctxt().push_adt(definition, name, AdtKind::Enum);
        self.hir.insert_adt(definition, adt);
        self.enum_items.push((item_id, definition));
        if let ItemKind::Enum(declaration) = &self.ast.item(item_id).kind {
            self.register_generics(definition, &declaration.generics, None, false);
            let count = self.own_generic_count(definition) as u32;
            self.hir.ty_ctxt().set_generic_count(adt, count);
            self.queue_bounds(
                definition,
                &declaration.generics,
                &[],
                &declaration.where_clause,
                None,
            );
        }

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
            .help("give each struct, enum, and trait a distinct name")
            .emit();
        } else {
            self.types.insert(name, definition);
        }
    }
}
