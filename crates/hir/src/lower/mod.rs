//! AST lowering and name resolution.

use seal_ast::{
    Ast, BlockId as AstBlockId, ImplDecl, ItemId, ItemKind, ParamId, TypeId as AstTypeId, TypeKind,
};
use seal_def::{DefData, DefId, DefKind};
use seal_diagnostics::{DiagCtxt, ErrorBuilder};
use seal_error_codes as codes;
use seal_error_codes::ErrorCode;
use seal_hash::FxHashMap;
use seal_span::Span;
use seal_symbol::{Interner, Symbol};
use seal_ty::{AdtKind, RefKind, TyCtxt, TyId, TyKind};

use crate::body::{Body, ParamData};
use crate::res::Res;
use crate::{Block, Hir, HirBlockId, ReceiverKind};

mod aggregates;
mod enums;
mod expr;
mod patterns;
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
    function_item_of: FxHashMap<DefId, ItemId>,
    function_items: Vec<ItemId>,
    struct_items: Vec<(ItemId, DefId)>,
    enum_items: Vec<(ItemId, DefId)>,
    traits: FxHashMap<Symbol, DefId>,
    trait_methods: FxHashMap<(DefId, Symbol), TraitMethodSig>,
    trait_method_order: FxHashMap<DefId, Vec<Symbol>>,
    pending_trait_impls: Vec<PendingTraitImpl>,
    /// The interned `Self` symbol, compared by identity in type syntax.
    self_symbol: Symbol,
    /// The implementing type while a trait signature is lowered.
    self_ty: Option<TyId>,
    function: Option<DefId>,
    loop_depth: usize,
}

/// One method signature declared by a trait.
///
/// The parameter and return types stay in the AST because they name `Self`,
/// which only has a meaning per implementation. Validating an impl re-lowers
/// them with the concrete type bound to [`Lowering::self_ty`].
#[derive(Clone, Debug)]
struct TraitMethodSig {
    name: Symbol,
    name_span: Span,
    receiver: Option<ReceiverKind>,
    /// Parameter types after the receiver, in declaration order.
    params: Vec<(AstTypeId, Span)>,
    /// The declared return type, or `None` for unit.
    ret: Option<AstTypeId>,
}

/// One `impl Trait for Type` block waiting for signature validation.
struct PendingTraitImpl {
    trait_def: DefId,
    type_def: DefId,
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
            pending_trait_impls: Vec::new(),
            self_symbol: interner.intern("Self"),
            self_ty: None,
            function: None,
            loop_depth: 0,
        }
    }

    fn run(mut self) -> Hir {
        self.collect_items();
        self.validate_trait_impls();
        self.lower_structs();
        self.lower_enums();
        self.validate_aggregates();
        self.lower_bodies();
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
            if declaration.trait_name.is_some() {
                self.collect_trait_impl(&declaration, span);
            } else {
                self.collect_inherent_impl(&declaration);
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

        let methods = match &self.ast.item(item_id).kind {
            ItemKind::Trait(declaration) => declaration.methods.clone(),
            _ => Vec::new(),
        };
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

        let (receiver, parameters) = self.trait_signature(trait_def, &params);
        self.trait_methods.insert(
            (trait_def, name),
            TraitMethodSig {
                name,
                name_span,
                receiver,
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
        match self.ast.ty(ty).kind {
            TypeKind::Path(symbol) if symbol == self.self_symbol => Some(ReceiverKind::Value),
            TypeKind::Pointer { pointee, mutable } => {
                let TypeKind::Path(symbol) = self.ast.ty(pointee).kind else {
                    return None;
                };
                if symbol != self.self_symbol {
                    return None;
                }
                Some(if mutable {
                    ReceiverKind::Exclusive
                } else {
                    ReceiverKind::Shared
                })
            }
            _ => None,
        }
    }

    /// Registers every function in one inherent impl block under its type.
    fn collect_inherent_impl(&mut self, declaration: &ImplDecl) {
        let Some(type_def) = self.impl_target(declaration) else {
            return;
        };
        for &item_id in &declaration.functions {
            self.collect_inherent_function(item_id, type_def);
        }
    }

    /// Resolves the nominal type an impl block targets.
    fn impl_target(&mut self, declaration: &ImplDecl) -> Option<DefId> {
        let Some(&type_def) = self.types.get(&declaration.type_name) else {
            let name = self.interner.resolve(declaration.type_name);
            if self.traits.contains_key(&declaration.type_name) {
                self.error(
                    codes::E2027,
                    declaration.type_span,
                    format!("`{name}` is a trait, not a type"),
                )
                .help("write `impl TraitName for TypeName` to implement a trait")
                .emit();
            } else if self.globals.contains_key(&declaration.type_name) {
                self.error(
                    codes::E2015,
                    declaration.type_span,
                    format!("`{name}` is a value, not a type"),
                )
                .help("an impl block attaches methods to a struct or enum")
                .emit();
            } else {
                self.error(
                    codes::E2015,
                    declaration.type_span,
                    format!("cannot find type `{name}` for this impl"),
                )
                .help("declare it with `struct` or `enum`, then implement it")
                .emit();
            }
            return None;
        };
        Some(type_def)
    }

    /// Registers every function in one trait impl block.
    fn collect_trait_impl(&mut self, declaration: &ImplDecl, span: Span) {
        let Some((trait_name, trait_span)) = declaration.trait_name else {
            return;
        };
        let Some(&trait_def) = self.traits.get(&trait_name) else {
            let name = self.interner.resolve(trait_name);
            if self.types.contains_key(&trait_name) {
                self.error(
                    codes::E2020,
                    trait_span,
                    format!("`{name}` is a type, not a trait"),
                )
                .help("write `impl Type { ... }` for an inherent implementation")
                .emit();
            } else {
                self.error(
                    codes::E2020,
                    trait_span,
                    format!("cannot find trait `{name}` for this impl"),
                )
                .help("declare the trait with `trait`, then implement it")
                .emit();
            }
            return;
        };
        let Some(type_def) = self.impl_target(declaration) else {
            return;
        };

        if let Some(first) = self.hir.trait_impl_span(trait_def, type_def) {
            self.error(
                codes::E2021,
                span,
                format!(
                    "the trait `{}` is already implemented for `{}`",
                    self.interner.resolve(trait_name),
                    self.interner.resolve(declaration.type_name)
                ),
            )
            .secondary(first, "first implemented here")
            .help("remove this block, or merge its functions into the first one")
            .emit();
        }

        let mut functions: Vec<(Symbol, DefId)> = Vec::new();
        let mut pending = Vec::with_capacity(declaration.functions.len());
        for &item_id in &declaration.functions {
            let Some((name, definition)) = self.define_impl_function(item_id, type_def) else {
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

        if self.hir.trait_impl_span(trait_def, type_def).is_none() {
            let map: FxHashMap<Symbol, DefId> = functions.iter().copied().collect();
            self.hir.insert_trait_impl(trait_def, type_def, map, span);
        }
        self.pending_trait_impls.push(PendingTraitImpl {
            trait_def,
            type_def,
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
        type_def: DefId,
    ) -> Option<(Symbol, DefId)> {
        let ItemKind::Func(declaration) = &self.ast.item(item_id).kind else {
            return None;
        };
        let (name, name_span) = (declaration.name, declaration.name_span);
        let span = self.ast.item(item_id).span;
        let params = declaration.params.clone();

        let definition = self.define(DefData {
            kind: DefKind::Function,
            name,
            name_span,
            span,
            parent: Some(type_def),
            mutable: false,
        });
        self.function_defs.insert(item_id, definition);
        self.function_item_of.insert(definition, item_id);
        self.function_items.push(item_id);

        let Some(receiver_param) = params.first().copied() else {
            return Some((name, definition));
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
                    let span = self.ast.param(param).name_span;
                    self.error(codes::E2016, span, "`self` must be the first parameter")
                        .help("move `self` to the first position, or rename the parameter")
                        .emit();
                }
            }
        }
        Some((name, definition))
    }

    /// Registers one inherent impl function under its type.
    fn collect_inherent_function(&mut self, item_id: ItemId, type_def: DefId) {
        let Some((name, definition)) = self.define_impl_function(item_id, type_def) else {
            return;
        };
        if let Some(first) = self.hir.method(type_def, name) {
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
            let Some(order) = self
                .trait_method_order
                .get(&implementation.trait_def)
                .cloned()
            else {
                continue;
            };
            let Some(adt) = self.hir.adt(implementation.type_def) else {
                continue;
            };
            let self_ty = self.hir.ty_ctxt().adt_ty(adt);
            let type_name = self
                .interner
                .resolve(self.hir.definitions().name(implementation.type_def))
                .to_owned();
            let trait_name = self
                .interner
                .resolve(self.hir.definitions().name(implementation.trait_def))
                .to_owned();

            let mut implemented: FxHashMap<Symbol, DefId> = FxHashMap::default();
            for &(function, name) in &implementation.functions {
                implemented.entry(name).or_insert(function);
            }

            for name in &order {
                let Some(signature) = self
                    .trait_methods
                    .get(&(implementation.trait_def, *name))
                    .cloned()
                else {
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
                            &signature,
                            function,
                            self_ty,
                            &trait_name,
                            &type_name,
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
        signature: &TraitMethodSig,
        function: DefId,
        self_ty: TyId,
        trait_name: &str,
        type_name: &str,
    ) {
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

        let receiver = self.hir.receiver(function);
        let mut implemented = Vec::with_capacity(params.len());
        for &param in &params {
            if self.is_self_parameter(param) {
                continue;
            }
            let data = self.ast.param(param);
            implemented.push((data.ty, data.span));
        }

        let saved = self.self_ty.replace(self_ty);
        let mut difference = if signature.receiver != receiver {
            Some("the receiver does not match the trait declaration".to_owned())
        } else if signature.params.len() != implemented.len() {
            Some(format!(
                "the trait declares {} parameter(s), but this function has {}",
                signature.params.len(),
                implemented.len()
            ))
        } else {
            self.compare_parameter_types(&signature.params, &implemented)
        };
        if difference.is_none() {
            difference = self.compare_return_type(signature.ret, ret);
        }
        self.self_ty = saved;

        let Some(difference) = difference else {
            return;
        };
        self.error(
            codes::E2024,
            name_span,
            format!(
                "`{}` does not match its declaration in the trait `{trait_name}`: {difference}",
                self.interner.resolve(signature.name)
            ),
        )
        .secondary(signature.name_span, "declared here")
        .help(format!(
            "align the receiver, parameters, and return type; `{type_name}` plays the role of `Self`"
        ))
        .emit();
    }

    fn compare_parameter_types(
        &mut self,
        declared: &[(AstTypeId, Span)],
        implemented: &[(AstTypeId, Span)],
    ) -> Option<String> {
        for (index, ((declared_ty, _), (implemented_ty, _))) in
            declared.iter().zip(implemented).enumerate()
        {
            let expected = self.lower_type(*declared_ty);
            let found = self.lower_type(*implemented_ty);
            if !self.trait_types_compatible(expected, found) {
                return Some(format!(
                    "parameter {} has type `{}`, but the trait declares `{}`",
                    index + 1,
                    self.display_ty(found),
                    self.display_ty(expected)
                ));
            }
        }
        None
    }

    fn compare_return_type(
        &mut self,
        declared: Option<AstTypeId>,
        implemented: Option<AstTypeId>,
    ) -> Option<String> {
        let expected = match declared {
            Some(ty) => self.lower_type(ty),
            None => self.hir.ty_ctxt().unit_ty(),
        };
        let found = match implemented {
            Some(ty) => self.lower_type(ty),
            None => self.hir.ty_ctxt().unit_ty(),
        };
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

    fn receiver_kind(&mut self, ty: seal_ast::TypeId, type_def: DefId) -> Option<ReceiverKind> {
        let adt = self.hir.adt(type_def)?;
        let target = self.hir.ty_ctxt().adt_ty(adt);
        let receiver = self.lower_type(ty);
        match self.hir.ty_ctxt().kind(receiver) {
            TyKind::Adt { adt: found, .. } if found == adt => Some(ReceiverKind::Value),
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
        let adt = self
            .hir
            .ty_ctxt()
            .push_adt(definition, name, AdtKind::Struct);
        self.hir.insert_adt(definition, adt);
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
