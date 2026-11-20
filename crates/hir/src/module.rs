//! Packages, modules, imports, and the public API surface.
//!
//! A package owns a tree of modules rooted at an empty path. A source file
//! contributes its items to exactly one module: the path declared by its first
//! `module` item, or the package root when it declares none. Several files may
//! contribute to one module; they are fragments of the same namespace.
//!
//! Imports are file-scoped. Two fragments of one module therefore see different
//! imported names, which keeps a file's meaning independent of what another
//! file happened to import. A cycle between modules or files is legal: modules
//! are collected before any import is followed.
//!
//! [`ApiTable`] records the modules a package exposes. A `public` item is part
//! of the API only when its module, and every ancestor of that module, is
//! exposed.

use seal_ast::ItemKind;
use seal_def::{DefId, ModuleId, PackageId, PackageInstanceId, Visibility};
use seal_diagnostics::DiagCtxt;
use seal_error_codes as codes;
use seal_hash::{FxHashMap, FxHashSet};
use seal_index::{Idx, IndexVec};
use seal_span::{FileId, Span};
use seal_symbol::{Interner, Symbol};

/// An absolute module path inside one package.
///
/// The package root is the empty path. Paths are compared segment by segment;
/// no lookup table or interning is needed because a module tree is small and
/// the path is stored once per module.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModulePath {
    segments: Vec<Symbol>,
}

impl ModulePath {
    /// Returns the package root path.
    #[inline]
    pub fn root() -> Self {
        Self::default()
    }

    /// Creates a path from its segments.
    #[inline]
    pub fn new(segments: Vec<Symbol>) -> Self {
        Self { segments }
    }

    /// Returns the segments, outermost first.
    #[inline]
    pub fn segments(&self) -> &[Symbol] {
        &self.segments
    }

    /// Returns the last segment, or `None` for the root.
    #[inline]
    pub fn name(&self) -> Option<Symbol> {
        self.segments.last().copied()
    }

    /// Returns `true` for the package root.
    #[inline]
    pub fn is_root(&self) -> bool {
        self.segments.is_empty()
    }

    /// Returns the parent path, or `None` for the root.
    #[inline]
    pub fn parent(&self) -> Option<Self> {
        (!self.segments.is_empty())
            .then(|| Self::new(self.segments[..self.segments.len() - 1].to_vec()))
    }

    /// Returns the path extended by one segment.
    #[inline]
    pub fn child(&self, name: Symbol) -> Self {
        let mut segments = self.segments.clone();
        segments.push(name);
        Self::new(segments)
    }

    /// Returns `true` when `self` is `prefix` or a descendant of it.
    pub fn starts_with(&self, prefix: &Self) -> bool {
        self.segments.starts_with(&prefix.segments)
    }

    /// Renders the dotted path used by symbols and diagnostics.
    ///
    /// The package root renders as `"root"` so it can appear in a symbol
    /// without an empty component.
    pub fn display(&self, interner: &Interner) -> String {
        if self.segments.is_empty() {
            return "root".to_owned();
        }
        self.segments
            .iter()
            .map(|segment| interner.resolve(*segment))
            .collect::<Vec<_>>()
            .join(".")
    }
}

/// One module in the package tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleData {
    /// The module identity.
    pub id: ModuleId,
    /// The package that owns the module.
    pub package: PackageId,
    /// The enclosing module, or `None` for the package root.
    pub parent: Option<ModuleId>,
    /// The last path segment, or `None` for the package root.
    pub name: Option<Symbol>,
    /// The absolute path inside the package.
    pub path: ModulePath,
    /// Whether the module is part of the package's public API.
    pub exposed: bool,
    /// The span of the declaration that introduced the module.
    pub declaration_span: Span,
}

/// One source file that contributes items to a module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleFragment {
    /// The module the file contributes to.
    pub module: ModuleId,
    /// The file.
    pub file: FileId,
    /// The span of the `module` declaration, or the file's first item when the
    /// file declares no module.
    pub span: Span,
}

/// One dependency edge from a package to another package.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackageDependency {
    /// The local name that imports use for the dependency.
    pub alias: Symbol,
    /// The package being imported.
    pub package: PackageId,
    /// The instance of the dependency compiled for this edge.
    pub instance: PackageInstanceId,
}

/// One package in the compilation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageData {
    /// The package identity.
    pub id: PackageId,
    /// The compiled instance.
    pub instance: PackageInstanceId,
    /// The declared package name.
    pub name: Symbol,
    /// The package root module.
    pub root: ModuleId,
    /// Every module, in creation order with the root first.
    pub modules: Vec<ModuleId>,
    /// Direct dependencies, in plan order.
    pub dependencies: Vec<PackageDependency>,
}

/// One source file in a package plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackagePlanFile {
    /// The file to compile.
    pub file: FileId,
    /// Whether the file is the package entry point.
    pub primary: bool,
}

/// The package part of a compilation plan.
///
/// This is the subset of the eventual `CompilePlan` that name resolution needs:
/// package identities, file membership, API exposure, and dependency aliases.
/// The project layer that builds a full plan belongs to a later phase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackagePlan {
    /// Every package, in dependency order. A package may only depend on a
    /// package that appears before it.
    pub packages: Vec<PackagePlanEntry>,
    /// The package being compiled.
    pub primary: PackageId,
}

impl PackagePlan {
    /// Creates a plan for one package with one file.
    pub fn single(name: Symbol, file: FileId) -> Self {
        Self {
            packages: vec![PackagePlanEntry {
                name,
                instance: PackageInstanceId::from_usize(0),
                files: vec![PackagePlanFile {
                    file,
                    primary: true,
                }],
                exposed: vec![ModulePath::root()],
                dependencies: Vec::new(),
            }],
            primary: PackageId::from_usize(0),
        }
    }
}

/// One package entry in a [`PackagePlan`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackagePlanEntry {
    /// The declared package name.
    pub name: Symbol,
    /// The compiled instance.
    pub instance: PackageInstanceId,
    /// The package's source files.
    pub files: Vec<PackagePlanFile>,
    /// Modules exposed by `[package.api]`.
    pub exposed: Vec<ModulePath>,
    /// Dependency aliases.
    pub dependencies: Vec<PackagePlanDependency>,
}

/// One dependency alias in a package plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackagePlanDependency {
    /// The name imports use for the dependency.
    pub alias: Symbol,
    /// The dependency package.
    pub package: PackageId,
}

/// The namespace a top-level name belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Namespace {
    /// Functions, constants, statics, and external declarations.
    Value,
    /// Structs, enums, and opaque types.
    Type,
    /// Traits.
    Trait,
}

/// What one import brings into scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportTarget {
    /// A module, usable as the first segment of another path.
    Module {
        /// The imported module.
        module: ModuleId,
    },
    /// One item of a module.
    Item {
        /// The module that declares the item.
        module: ModuleId,
        /// The declared name.
        name: Symbol,
        /// The item's namespace.
        namespace: Namespace,
    },
}

/// One resolved import binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportBinding {
    /// What the local name denotes.
    pub target: ImportTarget,
    /// The span of the whole `use` declaration.
    pub span: Span,
    /// The span of the local name.
    pub name_span: Span,
    /// The declared visibility of the imported item. Module imports are
    /// `public` for an exposed module and `private` otherwise.
    pub visibility: Visibility,
}

/// The imports of one source file.
///
/// A table is keyed by the local name, so an alias is just another entry. The
/// order is kept for deterministic `unused_import` diagnostics.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportTable {
    bindings: FxHashMap<Symbol, ImportBinding>,
    order: Vec<(Symbol, Span)>,
}

impl ImportTable {
    /// Returns the binding for `name`, if the file imports it.
    #[inline]
    pub fn get(&self, name: Symbol) -> Option<&ImportBinding> {
        self.bindings.get(&name)
    }

    /// Returns every binding in source order.
    pub fn iter(&self) -> impl Iterator<Item = (Symbol, &ImportBinding)> + '_ {
        self.order
            .iter()
            .filter_map(|(name, _)| self.bindings.get(name).map(|binding| (*name, binding)))
    }

    /// Returns the number of bindings.
    #[inline]
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Returns `true` when the file imports nothing.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    pub(crate) fn insert(&mut self, name: Symbol, binding: ImportBinding) -> bool {
        if self.bindings.contains_key(&name) {
            return false;
        }
        self.order.push((name, binding.span));
        self.bindings.insert(name, binding);
        true
    }
}

/// The modules a package exposes and the items they make public.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApiTable {
    exposed: FxHashSet<ModuleId>,
    items: Vec<DefId>,
}

impl ApiTable {
    /// Returns `true` when `module` is part of the public API.
    #[inline]
    pub fn is_exposed(&self, module: ModuleId) -> bool {
        self.exposed.contains(&module)
    }

    /// Returns every exposed module, in ascending order.
    pub fn exposed_modules(&self) -> Vec<ModuleId> {
        let mut modules: Vec<ModuleId> = self.exposed.iter().copied().collect();
        modules.sort_unstable();
        modules
    }

    /// Returns every public item reachable through an exposed module.
    #[inline]
    pub fn items(&self) -> &[DefId] {
        &self.items
    }

    pub(crate) fn expose(&mut self, module: ModuleId) {
        self.exposed.insert(module);
    }

    pub(crate) fn push_item(&mut self, def: DefId) {
        self.items.push(def);
    }
}

/// A top-level declaration known to the module graph before lowering.
///
/// The graph needs names and visibility to validate imports before any
/// [`DefId`] exists. The lowering pass then creates the definitions and reports
/// duplicates against these declarations.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ItemInfo {
    pub(crate) namespace: Namespace,
    pub(crate) visibility: Visibility,
    pub(crate) span: Span,
}

/// The module tree, file membership, and pre-lowering item names.
#[derive(Debug)]
pub(crate) struct ModuleGraph {
    pub(crate) modules: IndexVec<ModuleId, ModuleData>,
    pub(crate) packages: IndexVec<PackageId, PackageData>,
    pub(crate) fragments: Vec<ModuleFragment>,
    pub(crate) file_module: FxHashMap<FileId, ModuleId>,
    pub(crate) file_package: FxHashMap<FileId, PackageId>,
    pub(crate) children: FxHashMap<(ModuleId, Symbol), ModuleId>,
    pub(crate) items: FxHashMap<(ModuleId, Symbol), ItemInfo>,
    pub(crate) api: ApiTable,
}

impl ModuleGraph {
    /// Builds the module tree from a plan and the parsed files.
    pub(crate) fn build(
        plan: &PackagePlan,
        asts: &[(FileId, &seal_ast::Ast)],
        dcx: &DiagCtxt,
        interner: &Interner,
    ) -> Self {
        let mut graph = Self {
            modules: IndexVec::new(),
            packages: IndexVec::new(),
            fragments: Vec::new(),
            file_module: FxHashMap::default(),
            file_package: FxHashMap::default(),
            children: FxHashMap::default(),
            items: FxHashMap::default(),
            api: ApiTable::default(),
        };
        let ast_of: FxHashMap<FileId, &seal_ast::Ast> =
            asts.iter().map(|(file, ast)| (*file, *ast)).collect();

        for (index, entry) in plan.packages.iter().enumerate() {
            let package_id = PackageId::from_usize(index);
            let root = graph.create_root(package_id);
            graph.packages.push(PackageData {
                id: package_id,
                instance: entry.instance,
                name: entry.name,
                root,
                modules: vec![root],
                dependencies: Vec::new(),
            });

            for file in &entry.files {
                let Some(ast) = ast_of.get(&file.file) else {
                    dcx.bug(format!(
                        "the package plan names an unparsed file (file {})",
                        file.file.index()
                    ));
                };
                let module = graph.module_of_file(package_id, ast, dcx, interner);
                graph.file_module.insert(file.file, module);
                graph.file_package.insert(file.file, package_id);
                graph.fragments.push(ModuleFragment {
                    module,
                    file: file.file,
                    span: module_declaration_span(ast),
                });
                graph.collect_item_names(module, ast);
            }

            for dependency in &entry.dependencies {
                let package = dependency.package;
                if u32::from(package) >= index as u32 {
                    dcx.bug("a package plan lists a dependency that does not precede the package");
                }
                let dependency_data = PackageDependency {
                    alias: dependency.alias,
                    package,
                    instance: graph.packages[package].instance,
                };
                graph.packages[package_id]
                    .dependencies
                    .push(dependency_data);
                if let Some(module) = graph.children.get(&(root, dependency.alias)).copied() {
                    dcx.error(format!(
                        "the dependency alias `{}` conflicts with a top-level module",
                        interner.resolve(dependency.alias)
                    ))
                    .code(codes::E2077.diag_code())
                    .primary(
                        graph.modules[module].declaration_span,
                        "the module is declared here",
                    )
                    .help("rename the dependency alias, or the module")
                    .emit();
                }
            }

            for path in &entry.exposed {
                graph.expose_package_module(package_id, path, dcx, interner);
            }
        }

        graph
    }

    fn create_root(&mut self, package: PackageId) -> ModuleId {
        let id = ModuleId::from_usize(self.modules.len());
        self.modules.push(ModuleData {
            id,
            package,
            parent: None,
            name: None,
            path: ModulePath::root(),
            exposed: false,
            declaration_span: Span::default(),
        });
        id
    }

    fn push_module(&mut self, parent: ModuleId, name: Symbol, span: Span) -> ModuleId {
        let package = self.modules[parent].package;
        let path = self.modules[parent].path.child(name);
        let id = ModuleId::from_usize(self.modules.len());
        self.modules.push(ModuleData {
            id,
            package,
            parent: Some(parent),
            name: Some(name),
            path,
            exposed: false,
            declaration_span: span,
        });
        self.packages[package].modules.push(id);
        id
    }

    /// Returns the module of one file, creating implicit ancestors.
    fn module_of_file(
        &mut self,
        package: PackageId,
        ast: &seal_ast::Ast,
        dcx: &DiagCtxt,
        interner: &Interner,
    ) -> ModuleId {
        let root = self.packages[package].root;
        let mut declaration = None;
        for (position, &item_id) in ast.program().items().iter().enumerate() {
            let item = ast.item(item_id);
            let ItemKind::Module(module) = &item.kind else {
                continue;
            };
            if declaration.is_none() {
                declaration = Some((position, module.clone(), item.span));
            } else {
                dcx.error("a file may declare at most one module")
                    .code(codes::E2069.diag_code())
                    .primary(item.span, "this module declaration is redundant")
                    .help("keep the declaration at the top of the file and remove the others")
                    .emit();
            }
        }

        let Some((position, declaration, span)) = declaration else {
            return root;
        };
        if position != 0 {
            dcx.error("the module declaration must be the first item in the file")
                .code(codes::E2069.diag_code())
                .primary(span, "this declaration comes after another item")
                .help("move `module ...;` to the top of the file")
                .emit();
            return root;
        }
        if declaration.path.is_empty() {
            dcx.error("a module declaration needs at least one path segment")
                .code(codes::E2069.diag_code())
                .primary(declaration.path_span, "the path is empty")
                .help("write a path such as `module geometry::shapes;`")
                .emit();
            return root;
        }

        let mut current = root;
        for segment in &declaration.path {
            if interner.resolve(*segment) == "root" {
                dcx.error("`root` is reserved for the package root")
                    .code(codes::E2075.diag_code())
                    .primary(declaration.path_span, "this segment is reserved")
                    .help("choose a different module name")
                    .emit();
                return root;
            }
            current = match self.children.get(&(current, *segment)).copied() {
                Some(module) => module,
                None => {
                    let module = self.push_module(current, *segment, span);
                    self.children.insert((current, *segment), module);
                    module
                }
            };
        }
        current
    }

    /// Records every top-level item's namespace and visibility.
    fn collect_item_names(&mut self, module: ModuleId, ast: &seal_ast::Ast) {
        for &item_id in ast.program().items() {
            let item = ast.item(item_id);
            let (name, namespace) = match &item.kind {
                ItemKind::Struct(declaration) => (declaration.name, Namespace::Type),
                ItemKind::Enum(declaration) => (declaration.name, Namespace::Type),
                ItemKind::OpaqueType(declaration) => (declaration.name, Namespace::Type),
                ItemKind::Trait(declaration) => (declaration.name, Namespace::Trait),
                ItemKind::Func(declaration) => (declaration.name, Namespace::Value),
                ItemKind::Const(declaration) => (declaration.name, Namespace::Value),
                ItemKind::Static(declaration) => (declaration.name, Namespace::Value),
                ItemKind::ExternFunc(declaration) => (declaration.name, Namespace::Value),
                ItemKind::ExternStatic(declaration) => (declaration.name, Namespace::Value),
                ItemKind::Module(_) | ItemKind::Use(_) | ItemKind::Impl(_) => continue,
                _ => continue,
            };
            self.items.entry((module, name)).or_insert(ItemInfo {
                namespace,
                visibility: visibility_from(item.modifiers.visibility),
                span: item.span,
            });
        }
    }

    /// Marks one module and every ancestor as exposed.
    fn expose_package_module(
        &mut self,
        package: PackageId,
        path: &ModulePath,
        dcx: &DiagCtxt,
        interner: &Interner,
    ) {
        let mut current = self.packages[package].root;
        for segment in path.segments() {
            let Some(next) = self.children.get(&(current, *segment)).copied() else {
                dcx.error(format!(
                    "the public API exposes the module `{}`, which does not exist",
                    path.display(interner)
                ))
                .code(codes::E2081.diag_code())
                .primary(
                    self.modules[current].declaration_span,
                    "inside this package",
                )
                .help("declare the module with `module ...;` or remove it from the API")
                .emit();
                return;
            };
            current = next;
        }
        // A leaf keeps every ancestor reachable, but the root is part of the
        // API only when it is listed explicitly. Exposing `app` must not make
        // root items public just because `app` is a child of the root.
        let mut exposed = current;
        loop {
            if !self.modules[exposed].path.is_root() || path.is_root() {
                self.api.expose(exposed);
                self.modules[exposed].exposed = true;
            }
            match self.modules[exposed].parent {
                Some(parent) => exposed = parent,
                None => break,
            }
        }
    }
}

/// Converts the syntax visibility to the semantic one.
pub(crate) fn visibility_from(visibility: seal_ast::Visibility) -> Visibility {
    match visibility {
        seal_ast::Visibility::Private => Visibility::Private,
        seal_ast::Visibility::Internal => Visibility::Internal,
        seal_ast::Visibility::Public => Visibility::Public,
    }
}

/// Returns the span of a file's module declaration, or the whole file when it
/// declares none.
pub(crate) fn module_declaration_span(ast: &seal_ast::Ast) -> Span {
    ast.program()
        .items()
        .iter()
        .find_map(|&item_id| match &ast.item(item_id).kind {
            ItemKind::Module(declaration) => Some(declaration.path_span),
            _ => None,
        })
        .or_else(|| ast.program().items().first().map(|&id| ast.item(id).span))
        .unwrap_or_default()
}
