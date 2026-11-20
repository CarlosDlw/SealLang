//! AST lowering and name resolution.
//!
//! The pass is split by responsibility: [`items`] collects definitions and
//! checks duplicates, [`values`] lowers constant and static initializers,
//! [`bodies`] lowers function and default bodies, and this module owns the pass
//! state, the name-resolution stack, and the diagnostic helpers they share.

use seal_ast::{Ast, BlockId as AstBlockId, GenericParams, ItemId};
use seal_def::{DefData, DefId, DefKind};
use seal_diagnostics::{DiagCtxt, ErrorBuilder};
use seal_error_codes as codes;
use seal_error_codes::ErrorCode;
use seal_hash::FxHashMap;
use seal_span::Span;
use seal_symbol::{Interner, Symbol};
use seal_ty::{CAbiProperties, TyCtxt, TyId};

use crate::closure::ClosureEnv;
use crate::generics::GenericScopes;
use crate::res::Res;
use crate::{Block, Hir, HirBlockId, Protocol};

mod aggregates;
mod bodies;
mod closure;
mod coherence;
mod copy;
mod enums;
mod expr;
mod externs;
mod generics;
mod items;
mod object_safety;
mod opaque;
mod params;
mod paths;
mod patterns;
mod scope;
mod stmt;
mod structs;
mod supertraits;
mod types;
mod values;

pub(crate) use closure::CaptureContext;
pub(crate) use types::intern_primitives;

pub(crate) fn lower_ast(
    ast: &Ast,
    dcx: &DiagCtxt,
    interner: &Interner,
    c_abi: CAbiProperties,
) -> Hir {
    Lowering::new(ast, dcx, interner, c_abi).run()
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
    value_items: Vec<(ItemId, DefId)>,
    pub(super) extern_function_items: Vec<(ItemId, DefId)>,
    pub(super) extern_static_items: Vec<(ItemId, DefId)>,
    struct_items: Vec<(ItemId, DefId)>,
    opaque_items: Vec<(ItemId, DefId)>,
    enum_items: Vec<(ItemId, DefId)>,
    traits: FxHashMap<Symbol, DefId>,
    trait_methods: FxHashMap<(DefId, Symbol), items::TraitMethodSig>,
    trait_method_order: FxHashMap<DefId, Vec<Symbol>>,
    trait_items: Vec<(ItemId, DefId)>,
    trait_method_items: Vec<(ItemId, DefId)>,
    impl_items: Vec<(ItemId, DefId)>,
    impl_self_tys: FxHashMap<DefId, TyId>,
    impl_self_ty_of_function: FxHashMap<DefId, TyId>,
    pending_trait_impls: Vec<items::PendingTraitImpl>,
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
    /// The closures whose bodies are currently being lowered, outermost first.
    ///
    /// One frame is pushed per closure. The top frame receives every capture the
    /// body mentions; outer frames receive a value too when a nested closure
    /// needs to read it through them.
    closure_env: Vec<ClosureEnv>,
    /// The enclosing frame while a local static initializer is lowered.
    ///
    /// Resolution reports `E3060` instead of silently binding a name from the
    /// frame, because the static outlives every call to that function.
    capture: Option<CaptureContext>,
}

/// One owner's bounds waiting for every type definition to be collected.
struct PendingBounds {
    owner: DefId,
    params: GenericParams,
    supertraits: Vec<seal_ast::TraitPath>,
    where_clause: seal_ast::WhereClause,
    self_ty: Option<TyId>,
}

impl<'a> Lowering<'a> {
    fn new(ast: &'a Ast, dcx: &'a DiagCtxt, interner: &'a Interner, c_abi: CAbiProperties) -> Self {
        let hir = Hir::empty(TyCtxt::with_c_abi(c_abi), interner);
        let mut traits = FxHashMap::default();
        for protocol in Protocol::ALL {
            let definition = hir.protocol_trait(protocol);
            traits.insert(hir.definitions().name(definition), definition);
        }
        let drop_trait = hir.drop_trait();
        traits.insert(hir.definitions().name(drop_trait), drop_trait);
        let copy_trait = hir.copy_trait();
        traits.insert(hir.definitions().name(copy_trait), copy_trait);
        let c_void = hir.c_void();
        let c_void_name = hir.definitions().name(c_void);
        let mut types = FxHashMap::default();
        types.insert(c_void_name, c_void);
        Self {
            ast,
            dcx,
            interner,
            hir,
            scopes: scope::Scopes::default(),
            globals: FxHashMap::default(),
            types,
            function_defs: FxHashMap::default(),
            function_item_of: FxHashMap::default(),
            function_items: Vec::new(),
            value_items: Vec::new(),
            extern_function_items: Vec::new(),
            extern_static_items: Vec::new(),
            struct_items: Vec::new(),
            opaque_items: Vec::new(),
            enum_items: Vec::new(),
            traits,
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
            closure_env: Vec::new(),
            capture: None,
        }
    }

    fn run(mut self) -> Hir {
        self.collect_items();
        self.collect_extern_items();
        self.lower_pending_bounds();
        self.lower_structs();
        self.lower_opaque_types();
        self.lower_enums();
        self.lower_extern_declarations();
        self.validate_aggregates();
        self.build_trait_definitions();
        self.validate_supertraits();
        self.validate_coherence();
        self.validate_trait_impls();
        self.validate_copy_impls();
        self.validate_drop_impls();
        self.lower_value_bodies();
        self.lower_bodies();
        self.lower_trait_bodies();
        self.validate_dyn_types();
        self.hir
    }

    /// Resolves a name against the local scopes, then the function namespace.
    ///
    /// Inside a closure body, a name that belongs to an enclosing body becomes a
    /// capture. The innermost closure always receives the capture; outer closures
    /// receive it too when the value has to travel through their environments.
    pub(super) fn resolve(&mut self, name: Symbol, span: Span) -> Option<Res> {
        if let Some(definition) = self.scopes.lookup(name) {
            if let Some(barrier) = &self.capture {
                let parent = self.hir.definitions().get(definition).parent;
                if parent.is_some_and(|parent| barrier.forbids(parent)) {
                    self.error(
                        codes::E3060,
                        span,
                        format!(
                            "a local value initializer cannot capture `{}` from the enclosing function",
                            self.interner.resolve(name)
                        ),
                    )
                    .help("move the value into the declaration, or use a file-scope item")
                    .emit();
                    return None;
                }
            }
            let res = match self.hir.definitions().kind(definition) {
                DefKind::Const => Res::Const(definition),
                DefKind::Static => Res::Static(definition),
                DefKind::Param | DefKind::Binding => match self.capture_value(definition, span) {
                    Some((closure, index)) => Res::Capture { closure, index },
                    None => Res::Local(definition),
                },
                _ => Res::Local(definition),
            };
            return Some(res);
        }
        if let Some(&definition) = self.globals.get(&name) {
            let res = match self.hir.definitions().kind(definition) {
                DefKind::Const => Res::Const(definition),
                DefKind::Static | DefKind::ExternStatic => Res::Static(definition),
                _ => Res::Function(definition),
            };
            return Some(res);
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
