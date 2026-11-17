//! AST lowering and name resolution.

use seal_ast::{
    Ast, BlockId as AstBlockId, GenericParams, ImplDecl, ItemId, ItemKind, ParamId,
    TypeId as AstTypeId, TypeKind, WhereClause,
};
use seal_def::{DefData, DefId, DefKind};
use seal_diagnostics::{DiagCtxt, ErrorBuilder};
use seal_error_codes as codes;
use seal_error_codes::ErrorCode;
use seal_hash::FxHashMap;
use seal_span::Span;
use seal_symbol::{Interner, Symbol};
use seal_ty::{AdtKind, GenericSignature, RefKind, TraitRef, TyCtxt, TyId, TyKind};

use crate::body::{Body, ParamData};
use crate::generics::{GenericScopes, InherentImpl};
use crate::res::Res;
use crate::{Block, Hir, HirBlockId, ReceiverKind};

mod aggregates;
mod coherence;
mod enums;
mod expr;
mod generics;
mod object_safety;
mod paths;
mod patterns;
mod scope;
mod stmt;
mod structs;
mod supertraits;
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
    function_item_of: FxHashMap<DefId, ItemId>,
    function_items: Vec<ItemId>,
    struct_items: Vec<(ItemId, DefId)>,
    enum_items: Vec<(ItemId, DefId)>,
    traits: FxHashMap<Symbol, DefId>,
    trait_methods: FxHashMap<(DefId, Symbol), TraitMethodSig>,
    trait_method_order: FxHashMap<DefId, Vec<Symbol>>,
    trait_items: Vec<(ItemId, DefId)>,
    trait_method_items: Vec<(ItemId, DefId)>,
    impl_items: Vec<(ItemId, DefId)>,
    impl_self_tys: FxHashMap<DefId, TyId>,
    impl_self_ty_of_function: FxHashMap<DefId, TyId>,
    pending_trait_impls: Vec<PendingTraitImpl>,
    pending_bounds: Vec<PendingBounds>,
    /// Every declared parameter name, including owners no longer in scope.
    generic_param_names: FxHashMap<Symbol, Span>,
    generic_scopes: GenericScopes,
    /// The interned `Self` symbol, compared by identity in type syntax.
    self_symbol: Symbol,
    /// The concrete self type while an impl member is lowered.
    self_ty: Option<TyId>,
    function: Option<DefId>,
    loop_depth: usize,
}

/// One owner's bounds waiting for every type definition to be collected.
struct PendingBounds {
    owner: DefId,
    params: GenericParams,
    supertraits: Vec<seal_ast::TraitPath>,
    where_clause: WhereClause,
    self_ty: Option<TyId>,
}

/// One method signature declared by a trait.
///
/// The parameter and return types stay in the AST because they name `Self`,
/// which only has a meaning per implementation. Validating an impl re-lowers
/// them with the concrete type bound to [`Lowering::self_ty`].
#[derive(Clone, Debug)]
struct TraitMethodSig {
    def: DefId,
    name: Symbol,
    name_span: Span,
    receiver: Option<ReceiverKind>,
    /// Whether the declaration had a `self` parameter, even if it was invalid.
    has_self_parameter: bool,
    /// Parameter types after the receiver, in declaration order.
    params: Vec<(AstTypeId, Span)>,
    /// The declared return type, or `None` for unit.
    ret: Option<AstTypeId>,
}

/// One `impl Trait for Type` block waiting for signature validation.
struct PendingTraitImpl {
    trait_ref: TraitRef,
    self_ty: TyId,
    span: Span,
    /// The functions in source order, as `(definition, name)`.
    functions: Vec<(DefId, Symbol)>,
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
            function_item_of: FxHashMap::default(),
            function_items: Vec::new(),
            struct_items: Vec::new(),
            enum_items: Vec::new(),
            traits: FxHashMap::default(),
            trait_methods: FxHashMap::default(),
            trait_method_order: FxHashMap::default(),
            trait_items: Vec::new(),
            trait_method_items: Vec::new(),
            impl_items: Vec::new(),
            impl_self_tys: FxHashMap::default(),
            impl_self_ty_of_function: FxHashMap::default(),
            pending_trait_impls: Vec::new(),
            pending_bounds: Vec::new(),
            generic_param_names: FxHashMap::default(),
            generic_scopes: GenericScopes::default(),
            self_symbol: interner.intern("Self"),
            self_ty: None,
            function: None,
            loop_depth: 0,
        }
    }

    fn run(mut self) -> Hir {
        self.collect_items();
        self.lower_pending_bounds();
        self.lower_structs();
        self.lower_enums();
        self.validate_aggregates();
        self.build_trait_definitions();
        self.validate_supertraits();
        self.validate_coherence();
        self.validate_trait_impls();
        self.lower_bodies();
        self.validate_dyn_types();
        self.hir
    }

    /// Creates a definition for every item before any body is lowered, so calls
    /// and type annotations work regardless of declaration order.
    ///
    /// Collection runs in dependency order: structs, enums, traits, free
    /// functions, and finally impl blocks. An impl needs its target type and,
    /// when it names a trait, the trait declaration; a body that calls
    /// `Point::new` needs the impl registry to be complete, so bodies are
    /// lowered only after every pass.
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

        let levels = self.push_generic_scope(impl_def);
        let saved = self.self_ty.replace(self_ty);
        let trait_ref = declaration
            .trait_ref
            .as_ref()
            .and_then(|path| self.lower_impl_trait_ref(path, self_ty));
        self.self_ty = saved;
        self.pop_generic_scope(levels);
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
        let trait_def = trait_ref.trait_def;
        if let Some(type_def) = nominal {
            let map: FxHashMap<Symbol, DefId> = functions.iter().copied().collect();
            self.hir.insert_trait_impl(trait_def, type_def, map);
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
    fn validate_trait_impls(&mut self) {
        let pending = std::mem::take(&mut self.pending_trait_impls);
        for implementation in pending {
            let trait_def = implementation.trait_ref.trait_def;
            let Some(order) = self.trait_method_order.get(&trait_def).cloned() else {
                continue;
            };
            let Some(declaration) = self.hir.trait_definition(trait_def) else {
                continue;
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
                let Some(raw) = self.trait_methods.get(&(trait_def, *name)).cloned() else {
                    continue;
                };
                let Some(method) = declaration.method(*name) else {
                    continue;
                };
                match implemented.get(name) {
                    None => {
                        self.error(
                            codes::E2022,
                            implementation.span,
                            format!(
                                "the trait `{trait_name}` requires `{}`, which `{type_name}` does not implement",
                                self.interner.resolve(*name)
                            ),
                        )
                        .help(format!(
                            "add `func {}(...)` to this impl block",
                            self.interner.resolve(*name)
                        ))
                        .emit();
                    }
                    Some(&function) => {
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

    /// Compares one implementation function with its trait method.
    fn check_trait_method(
        &mut self,
        raw: &TraitMethodSig,
        declared: &seal_ty::TraitMethodDef,
        function: DefId,
        trait_ref: TraitRef,
        names: (&str, &str),
    ) {
        let (trait_name, type_name) = names;
        let self_ty = trait_ref.self_ty;
        let Some(&item_id) = self.function_item_of.get(&function) else {
            return;
        };
        let (params, ret, name_span) = {
            let ItemKind::Func(declaration) = &self.ast.item(item_id).kind else {
                return;
            };
            (
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

        let receiver = self.hir.receiver(function);
        let mut difference = if raw.receiver != receiver {
            Some("the receiver does not match the trait declaration".to_owned())
        } else if declared.signature.params.len() != implemented.len() {
            Some(format!(
                "the trait declares {} parameter(s), but this function has {}",
                declared.signature.params.len(),
                implemented.len()
            ))
        } else {
            match self.instantiate_trait_signature(declared, trait_ref, function) {
                Err(difference) => Some(difference),
                Ok(expected) => self.compare_parameter_types(&expected.params, &implemented),
            }
        };
        if difference.is_none() {
            if let Ok(expected) = self.instantiate_trait_signature(declared, trait_ref, function) {
                difference = self.compare_return_type(expected.ret, found_ret);
            }
        }

        let Some(difference) = difference else {
            return;
        };
        self.error(
            codes::E2024,
            name_span,
            format!(
                "`{}` does not match its declaration in the trait `{trait_name}`: {difference}",
                self.interner.resolve(raw.name)
            ),
        )
        .secondary(raw.name_span, "declared here")
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

    fn display_ty(&self, ty: TyId) -> String {
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
            let generic_levels = self.push_generic_scope(function);
            let saved_self = self.self_ty;
            self.self_ty = self.impl_self_ty_of_function.get(&function).copied();
            self.scopes.push();
            let ret = ret.map(|ty| self.lower_type(ty));
            let params = params
                .iter()
                .map(|param| self.lower_param(*param, function))
                .collect();
            let root = self.lower_block(body);
            self.scopes.pop();
            self.self_ty = saved_self;
            self.pop_generic_scope(generic_levels);

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
        if self.generic_scope_param(name).is_some() {
            self.error(
                codes::E2010,
                span,
                format!(
                    "`{}` is a type parameter, not a value",
                    self.interner.resolve(name)
                ),
            )
            .help("type parameters name types; a value must be supplied by the caller")
            .emit();
            return None;
        }
        if self.traits.contains_key(&name) {
            self.error(
                codes::E2027,
                span,
                format!("`{}` is a trait, not a value", self.interner.resolve(name)),
            )
            .help("call one of its methods on a value of an implementing type")
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
