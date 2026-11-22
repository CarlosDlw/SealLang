//! AST lowering and name resolution.
//!
//! The pass is split by responsibility: [`items`] collects definitions and
//! checks duplicates, [`values`] lowers constant and static initializers,
//! [`bodies`] lowers function and default bodies, and this module owns the pass
//! state, the name-resolution stack, and the diagnostic helpers they share.
//!
//! Lowering runs over every file of a package at once. Items are collected in
//! dependency order across files, so a function in one file may call a function
//! in another and a struct may name a struct declared elsewhere. Each item is
//! resolved in its own module: unqualified names consult the module namespace
//! and the imports of the file that declares the item, never a global map.

use std::cell::RefCell;

use seal_ast::{Ast, BlockId as AstBlockId, GenericParams, ItemId};
use seal_def::{DefData, DefId, DefKind, ModuleId, PackageId, PackageInstanceId};
use seal_diagnostics::{DiagCtxt, ErrorBuilder};
use seal_error_codes as codes;
use seal_error_codes::ErrorCode;
use seal_hash::{FxHashMap, FxHashSet};
use seal_span::{FileId, Span};
use seal_symbol::{Interner, Symbol};
use seal_ty::{CAbiProperties, TyCtxt, TyId};

use crate::closure::ClosureEnv;
use crate::generics::GenericScopes;
use crate::module::{ImportTable, Namespace};
use crate::res::Res;
use crate::{Block, Hir, HirBlockId, Protocol};

mod aggregates;
mod api;
mod bodies;
mod closure;
mod coherence;
mod copy;
mod enums;
mod expr;
mod externs;
mod generics;
mod imports;
mod items;
mod object_safety;
mod opaque;
mod params;
mod paths;
mod patterns;
mod prelude;
mod reexports;
mod scope;
mod stmt;
mod structs;
mod supertraits;
mod types;
mod values;

pub(crate) use closure::CaptureContext;
pub(crate) use types::intern_primitives;

/// One parsed file and the module it contributes to.
pub(crate) struct AstFile<'a> {
    pub(crate) file: FileId,
    pub(crate) ast: &'a Ast,
    pub(crate) module: ModuleId,
    pub(crate) package: PackageId,
    pub(crate) instance: PackageInstanceId,
}

/// A top-level item together with the file that declares it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ItemRef {
    pub(crate) file: usize,
    pub(crate) id: ItemId,
}

pub(crate) fn lower_ast(
    ast: &Ast,
    file: FileId,
    dcx: &DiagCtxt,
    interner: &Interner,
    c_abi: CAbiProperties,
) -> Hir {
    let name = interner.intern("main");
    let plan = crate::module::PackagePlan::single(name, file);
    let asts = [(file, ast)];
    lower_plan(&plan, &asts, dcx, interner, c_abi)
}

/// Lowers every file of a package plan into one HIR.
pub(crate) fn lower_package(
    plan: &crate::module::PackagePlan,
    asts: &[(FileId, Ast)],
    dcx: &DiagCtxt,
    interner: &Interner,
    c_abi: CAbiProperties,
) -> Hir {
    let views: Vec<(FileId, &Ast)> = asts.iter().map(|(file, ast)| (*file, ast)).collect();
    lower_plan(plan, &views, dcx, interner, c_abi)
}

fn lower_plan(
    plan: &crate::module::PackagePlan,
    asts: &[(FileId, &Ast)],
    dcx: &DiagCtxt,
    interner: &Interner,
    c_abi: CAbiProperties,
) -> Hir {
    crate::cfg::validate_env(plan, asts, dcx, interner);
    let mut graph = crate::module::ModuleGraph::build(plan, asts, dcx, interner);
    reexports::resolve_reexports(&mut graph, asts, dcx, interner);
    graph.resolve_deferred_exposures(dcx, interner);
    graph.preludes = prelude::resolve_preludes(plan, &graph, dcx, interner);
    let imports = imports::resolve_imports(&graph, asts, dcx, interner);
    let files: Vec<AstFile<'_>> = asts
        .iter()
        .map(|(file, ast)| {
            let module = graph.file_module[file];
            let package = graph.file_package[file];
            AstFile {
                file: *file,
                ast,
                module,
                package,
                instance: graph.packages[package].instance,
            }
        })
        .collect();
    Lowering::new(&files, graph, imports, plan.primary, dcx, interner, c_abi).run()
}

/// One lowering pass over every file of a package plan.
pub(crate) struct Lowering<'a> {
    pub(super) files: &'a [AstFile<'a>],
    pub(super) ast: &'a Ast,
    pub(super) current_file: usize,
    pub(super) current_module: ModuleId,
    pub(super) current_package: PackageId,
    pub(super) current_instance: PackageInstanceId,
    pub(super) dcx: &'a DiagCtxt,
    pub(super) interner: &'a Interner,
    pub(super) hir: Hir,
    pub(super) imports: Vec<ImportTable>,
    used_imports: RefCell<Vec<FxHashSet<Symbol>>>,
    scopes: scope::Scopes,
    pub(super) globals: FxHashMap<(ModuleId, Symbol), DefId>,
    pub(super) types: FxHashMap<(ModuleId, Symbol), DefId>,
    pub(super) traits: FxHashMap<(ModuleId, Symbol), DefId>,
    builtin_types: FxHashMap<Symbol, DefId>,
    builtin_traits: FxHashMap<Symbol, DefId>,
    function_item_of: FxHashMap<DefId, ItemRef>,
    function_items: Vec<(ItemRef, DefId)>,
    value_items: Vec<(ItemRef, DefId)>,
    pub(super) extern_function_items: Vec<(ItemRef, DefId)>,
    pub(super) extern_static_items: Vec<(ItemRef, DefId)>,
    struct_items: Vec<(ItemRef, DefId)>,
    opaque_items: Vec<(ItemRef, DefId)>,
    enum_items: Vec<(ItemRef, DefId)>,
    trait_methods: FxHashMap<(DefId, Symbol), items::TraitMethodSig>,
    trait_method_order: FxHashMap<DefId, Vec<Symbol>>,
    trait_items: Vec<(ItemRef, DefId)>,
    trait_method_items: Vec<(ItemRef, DefId)>,
    impl_items: Vec<(ItemRef, DefId)>,
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
    fn new(
        files: &'a [AstFile<'a>],
        graph: crate::module::ModuleGraph,
        imports: Vec<ImportTable>,
        primary_package: PackageId,
        dcx: &'a DiagCtxt,
        interner: &'a Interner,
        c_abi: CAbiProperties,
    ) -> Self {
        let imports_by_file: FxHashMap<FileId, ImportTable> = files
            .iter()
            .zip(imports.iter())
            .map(|(file, table)| (file.file, table.clone()))
            .collect();
        let mut hir = Hir::empty(TyCtxt::with_c_abi(c_abi), interner);
        hir.install_graph(
            graph.modules,
            graph.packages,
            graph.file_module,
            graph.api,
            graph.children,
            graph.exports,
            graph.preludes,
            imports_by_file,
            primary_package,
        );
        let mut builtin_traits = FxHashMap::default();
        for protocol in Protocol::ALL {
            let definition = hir.protocol_trait(protocol);
            builtin_traits.insert(hir.definitions().name(definition), definition);
        }
        let drop_trait = hir.drop_trait();
        builtin_traits.insert(hir.definitions().name(drop_trait), drop_trait);
        let copy_trait = hir.copy_trait();
        builtin_traits.insert(hir.definitions().name(copy_trait), copy_trait);
        for kind in seal_ty::ClosureKind::ALL {
            let definition = hir.fn_trait(kind);
            builtin_traits.insert(hir.definitions().name(definition), definition);
        }
        let c_void = hir.c_void();
        let mut builtin_types = FxHashMap::default();
        builtin_types.insert(hir.definitions().name(c_void), c_void);

        let first = &files[0];
        let (module, package, instance) = (first.module, first.package, first.instance);
        let import_count = imports.len();
        Self {
            files,
            ast: files[0].ast,
            current_file: 0,
            current_module: module,
            current_package: package,
            current_instance: instance,
            dcx,
            interner,
            hir,
            imports,
            used_imports: RefCell::new(vec![FxHashSet::default(); import_count]),
            scopes: scope::Scopes::default(),
            globals: FxHashMap::default(),
            types: FxHashMap::default(),
            traits: FxHashMap::default(),
            builtin_types,
            builtin_traits,
            function_item_of: FxHashMap::default(),
            function_items: Vec::new(),
            value_items: Vec::new(),
            extern_function_items: Vec::new(),
            extern_static_items: Vec::new(),
            struct_items: Vec::new(),
            opaque_items: Vec::new(),
            enum_items: Vec::new(),
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
        self.validate_api();
        self.validate_dyn_types();
        self.finish_imports();
        self.hir
    }

    /// Switches the pass to one file and its module.
    pub(super) fn enter_file(&mut self, index: usize) {
        self.current_file = index;
        self.ast = self.files[index].ast;
        let file = &self.files[index];
        self.current_module = file.module;
        self.current_package = file.package;
        self.current_instance = file.instance;
    }

    /// Returns one file's tree without changing the current file.
    pub(super) fn ast_of(&self, item: ItemRef) -> &'a Ast {
        self.files[item.file].ast
    }

    /// Returns whether the item's `cfg` condition is active in this instance.
    pub(super) fn item_enabled(&self, item_id: ItemId) -> bool {
        let modifiers = self.ast.item(item_id).modifiers;
        let cfg = &self.hir.package(self.current_package).cfg;
        // `test` and `bench` are configuration modifiers: the item exists only
        // in the matching harness build, so normal builds never type-check or
        // generate test bodies.
        if (modifiers.is_test && !cfg.test) || (modifiers.is_bench && !cfg.bench) {
            return false;
        }
        let Some(condition) = modifiers.cfg else {
            return true;
        };
        cfg.evaluate(self.ast, condition, self.interner)
    }

    /// Returns every top-level item of every file, in file order.
    pub(super) fn top_level_items(&self) -> Vec<ItemRef> {
        let mut items = Vec::new();
        for (file, source) in self.files.iter().enumerate() {
            let cfg = &self.hir.package(source.package).cfg;
            if !crate::cfg::module_enabled(source.ast, cfg, self.interner) {
                continue;
            }
            for &id in source.ast.program().items() {
                let modifiers = source.ast.item(id).modifiers;
                if (modifiers.is_test && !cfg.test) || (modifiers.is_bench && !cfg.bench) {
                    continue;
                }
                if modifiers
                    .cfg
                    .is_some_and(|condition| !cfg.evaluate(source.ast, condition, self.interner))
                {
                    continue;
                }
                items.push(ItemRef { file, id });
            }
        }
        items
    }

    /// Resolves a name against the local scopes, then the module namespaces.
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
        if let Some(definition) = self.value_def(name) {
            let res = match self.hir.definitions().kind(definition) {
                DefKind::Const => Res::Const(definition),
                DefKind::Static | DefKind::ExternStatic => Res::Static(definition),
                _ => Res::Function(definition),
            };
            return Some(res);
        }
        if self.type_def(name).is_some() {
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
        if self.trait_def(name).is_some() {
            self.error(
                codes::E2027,
                span,
                format!("`{}` is a trait, not a value", self.interner.resolve(name)),
            )
            .help("call one of its methods on a value of an implementing type")
            .emit();
            return None;
        }
        if self.module_named(name).is_some() {
            self.error(
                codes::E2086,
                span,
                format!("`{}` is a module, not a value", self.interner.resolve(name)),
            )
            .help("name an item inside the module")
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

    /// Returns the value declared as `name` in the current module or imported.
    pub(super) fn value_def(&self, name: Symbol) -> Option<DefId> {
        self.module_def(Namespace::Value, name)
            .or_else(|| self.imported(name, Namespace::Value))
            .or_else(|| self.prelude_def(name, Namespace::Value))
    }

    /// Returns the type declared as `name` in the current module or imported.
    pub(super) fn type_def(&self, name: Symbol) -> Option<DefId> {
        self.module_def(Namespace::Type, name)
            .or_else(|| self.imported(name, Namespace::Type))
            .or_else(|| self.prelude_def(name, Namespace::Type))
            .or_else(|| self.builtin_types.get(&name).copied())
    }

    /// Returns the trait declared as `name` in the current module or imported.
    pub(super) fn trait_def(&self, name: Symbol) -> Option<DefId> {
        self.module_def(Namespace::Trait, name)
            .or_else(|| self.imported(name, Namespace::Trait))
            .or_else(|| self.prelude_def(name, Namespace::Trait))
            .or_else(|| self.builtin_traits.get(&name).copied())
    }

    /// Resolves one unqualified name through the current package's prelude.
    fn prelude_def(&self, name: Symbol, namespace: Namespace) -> Option<DefId> {
        let binding = self
            .hir
            .prelude(self.current_package)?
            .binding(name, namespace)?;
        match namespace {
            Namespace::Value => self.globals.get(&(binding.module, binding.name)).copied(),
            Namespace::Type => self.types.get(&(binding.module, binding.name)).copied(),
            Namespace::Trait => self.traits.get(&(binding.module, binding.name)).copied(),
        }
    }

    /// Resolves one item directly in `module` or through a resolved re-export.
    pub(super) fn module_item(
        &self,
        module: ModuleId,
        name: Symbol,
        namespace: Namespace,
    ) -> Option<(DefId, Option<crate::module::ExportBinding>)> {
        let direct = match namespace {
            Namespace::Value => self.globals.get(&(module, name)).copied(),
            Namespace::Type => self.types.get(&(module, name)).copied(),
            Namespace::Trait => self.traits.get(&(module, name)).copied(),
        };
        if let Some(definition) = direct {
            return Some((definition, None));
        }
        let export = self.hir.module_export(module, name).copied()?;
        let crate::module::ExportTarget::Item {
            module: target_module,
            name: target_name,
            namespace: target_namespace,
        } = export.target
        else {
            return None;
        };
        if target_namespace != namespace {
            return None;
        }
        let definition = match namespace {
            Namespace::Value => self.globals.get(&(target_module, target_name)).copied(),
            Namespace::Type => self.types.get(&(target_module, target_name)).copied(),
            Namespace::Trait => self.traits.get(&(target_module, target_name)).copied(),
        }?;
        Some((definition, Some(export)))
    }

    /// Checks that a re-export used by a qualified path is reachable here.
    pub(super) fn export_reachable(
        &mut self,
        export: Option<crate::module::ExportBinding>,
        span: Span,
    ) -> bool {
        let Some(export) = export else {
            return true;
        };
        let owner = export.module;
        let owner_package = self.hir.module(owner).package;
        if export.visibility == seal_def::Visibility::Internal
            && owner_package != self.current_package
        {
            self.error(
                codes::E2072,
                span,
                "the re-export is internal to its package",
            )
            .help("use a public re-export, or keep the path inside the package")
            .emit();
            return false;
        }
        if owner_package != self.current_package && !self.hir.module(owner).exposed {
            self.error(codes::E2080, span, "the re-export module is not exposed")
                .help("expose the module in `[package.api]`")
                .emit();
            return false;
        }
        true
    }

    /// Returns `true` when the current module or an import names a value.
    pub(super) fn value_exists(&self, name: Symbol) -> bool {
        self.value_def(name).is_some()
    }

    /// Returns `true` when the current module or an import names a type.
    pub(super) fn type_exists(&self, name: Symbol) -> bool {
        self.type_def(name).is_some()
    }

    /// Returns `true` when the current module or an import names a trait.
    pub(super) fn trait_exists(&self, name: Symbol) -> bool {
        self.trait_def(name).is_some()
    }

    /// Looks a name up in the current module only, ignoring imports.
    pub(super) fn local_value(&self, name: Symbol) -> Option<DefId> {
        self.globals.get(&(self.current_module, name)).copied()
    }

    /// Looks a type up in the current module only, ignoring imports.
    pub(super) fn local_type(&self, name: Symbol) -> Option<DefId> {
        self.types.get(&(self.current_module, name)).copied()
    }

    /// Looks a trait up in the current module only, ignoring imports.
    pub(super) fn local_trait(&self, name: Symbol) -> Option<DefId> {
        self.traits.get(&(self.current_module, name)).copied()
    }

    fn module_def(&self, namespace: Namespace, name: Symbol) -> Option<DefId> {
        match namespace {
            Namespace::Value => self.local_value(name),
            Namespace::Type => self.local_type(name),
            Namespace::Trait => self.local_trait(name),
        }
    }

    /// Resolves an unqualified name through the current file's imports.
    fn imported(&self, name: Symbol, namespace: Namespace) -> Option<DefId> {
        let table = self.imports.get(self.current_file)?;
        let binding = table.get(name)?;
        let crate::module::ImportTarget::Item {
            module,
            name: target,
            namespace: target_namespace,
        } = binding.target
        else {
            return None;
        };
        if target_namespace != namespace {
            return None;
        }
        let definition = match namespace {
            Namespace::Value => self.globals.get(&(module, target)).copied(),
            Namespace::Type => self.types.get(&(module, target)).copied(),
            Namespace::Trait => self.traits.get(&(module, target)).copied(),
        };
        if definition.is_some() {
            if let Some(used) = self.used_imports.borrow_mut().get_mut(self.current_file) {
                used.insert(name);
            }
        }
        definition
    }

    /// Resolves a module binding in the current file.
    pub(super) fn imported_module(&self, name: Symbol) -> Option<ModuleId> {
        let table = self.imports.get(self.current_file)?;
        let binding = table.get(name)?;
        let crate::module::ImportTarget::Module { module } = binding.target else {
            return None;
        };
        if let Some(used) = self.used_imports.borrow_mut().get_mut(self.current_file) {
            used.insert(name);
        }
        Some(module)
    }

    /// Emits `unused_import` for every binding the file never resolved.
    fn finish_imports(&mut self) {
        for (index, table) in self.imports.clone().iter().enumerate() {
            let used = self.used_imports.borrow();
            let used = used.get(index);
            for (name, binding) in table.iter() {
                if used.is_some_and(|used| used.contains(&name)) {
                    continue;
                }
                self.dcx
                    .warning(format!(
                        "the import `{}` is never used",
                        self.interner.resolve(name)
                    ))
                    .primary(binding.span, "this import is unused")
                    .help("remove it, or use the imported name")
                    .emit();
            }
        }
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

    /// Reports a duplicate declared by two different files.
    pub(super) fn duplicate(
        &mut self,
        first: DefId,
        name_span: Span,
        message: String,
        same_namespace: ErrorCode,
    ) {
        let first_span = self.hir.definitions().get(first).name_span;
        let first_file = self.hir.definitions().file(first);
        let file = self.files[self.current_file].file;
        let code = if first_file.is_some_and(|first_file| first_file != file) {
            codes::E2083
        } else {
            same_namespace
        };
        self.error(code, name_span, message)
            .secondary(first_span, "first defined here")
            .help("give each definition a distinct name")
            .emit();
    }

    /// Creates a definition in the current module.
    fn define(&mut self, mut data: DefData) -> DefId {
        data.package = self.current_package;
        data.instance = self.current_instance;
        data.module = self.current_module;
        data.file = Some(self.files[self.current_file].file);
        self.hir.define(data)
    }

    /// Creates a top-level item definition and records its declared visibility.
    pub(super) fn define_item(&mut self, item_id: ItemId, data: DefData) -> DefId {
        let modifiers = self.ast.item(item_id).modifiers;
        let definition = self.define(data);
        // The generated harness lives in the package root, outside the module
        // that declares a test. Making a test entry point internal under its
        // own configuration keeps it reachable without exposing it in normal
        // builds, where the harness does not exist.
        let harness_visible = (modifiers.is_test
            && self.hir.package(self.current_package).cfg.test)
            || (modifiers.is_bench && self.hir.package(self.current_package).cfg.bench);
        let visibility = if harness_visible {
            seal_ast::Visibility::Internal
        } else {
            modifiers.visibility
        };
        self.hir
            .definitions_mut()
            .set_visibility(definition, crate::module::visibility_from(visibility));
        self.hir
            .definitions_mut()
            .set_test(definition, modifiers.is_test);
        self.hir
            .definitions_mut()
            .set_bench(definition, modifiers.is_bench);
        self.hir
            .definitions_mut()
            .set_doc_span(definition, self.ast.item(item_id).docs);
        if let Some(condition) = self.ast.item(item_id).modifiers.cfg {
            let text = crate::cfg::format(self.ast, condition, self.interner);
            let symbol = self.interner.intern(&text);
            self.hir.definitions_mut().set_cfg(definition, Some(symbol));
        }
        definition
    }

    /// Creates a field definition and records its declared visibility.
    pub(super) fn define_field(
        &mut self,
        visibility: seal_ast::Visibility,
        data: DefData,
    ) -> DefId {
        let definition = self.define(data);
        self.hir
            .definitions_mut()
            .set_visibility(definition, crate::module::visibility_from(visibility));
        definition
    }

    /// Reports an implementation written outside the package that owns its type
    /// or trait.
    ///
    /// Returns `true` when the implementation is an orphan. The declaration is
    /// still lowered, so later diagnostics do not cascade from the missing
    /// implementation.
    pub(super) fn check_orphan_impl(
        &mut self,
        type_def: Option<DefId>,
        trait_def: Option<DefId>,
        span: Span,
    ) -> bool {
        let local_type =
            type_def.is_some_and(|def| self.hir.definitions().package(def) == self.current_package);
        let local_trait = trait_def
            .is_some_and(|def| self.hir.definitions().package(def) == self.current_package);
        if local_type || local_trait {
            return false;
        }
        let what = match (type_def, trait_def) {
            (_, Some(_)) => "neither the type nor the trait",
            _ => "the type",
        };
        self.error(
            codes::E2082,
            span,
            format!("this implementation is an orphan: {what} is declared in another package"),
        )
        .help("move the implementation into the package that declares it, or wrap the type in a local struct")
        .emit();
        true
    }

    /// Returns the item reference of a definition declared in the current file.
    pub(super) fn item_ref(&self, id: ItemId) -> ItemRef {
        ItemRef {
            file: self.current_file,
            id,
        }
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
