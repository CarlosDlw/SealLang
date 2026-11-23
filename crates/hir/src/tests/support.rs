//! Fixtures and structural checks for HIR tests.

use std::collections::BTreeSet;
use std::io;
use std::sync::{Arc, Mutex};

use crate::{
    Body, BodyId, BodyKind, ExprKind, Hir, HirBlockId, HirExprId, HirPatternId, HirStmtId,
    ModuleId, ModulePath, PackagePlan, PackagePlanDependency, PackagePlanEntry, PackagePlanFile,
    PatternKind, PreludeSource, Res, StmtKind,
};
use seal_ast::Ast;
use seal_def::{DefId, DefKind, PackageId, PackageInstanceId};
use seal_diagnostics::{DiagCtxt, Diagnostic, Emitter};
use seal_index::Idx;
use seal_span::{FileName, SourceFile, SourceMap, Span};
use seal_symbol::{Interner, Symbol};
use seal_ty::TyId;
use seal_version::Edition;

#[derive(Clone, Default)]
struct Recording(Arc<Mutex<Vec<Diagnostic>>>);

impl Emitter for Recording {
    fn emit(&mut self, diagnostic: &Diagnostic, _source_map: &SourceMap) -> io::Result<()> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(diagnostic.clone());
        Ok(())
    }
}

impl Recording {
    fn diagnostics(&self) -> Vec<Diagnostic> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
}

pub(crate) struct Fixture {
    map: Arc<SourceMap>,
    pub(crate) dcx: DiagCtxt,
    pub(crate) interner: Interner,
    pub(crate) file: Arc<SourceFile>,
    files: Vec<seal_span::FileId>,
    package_files: Vec<Vec<seal_span::FileId>>,
    packages: Vec<PackageSpec>,
    editions: Vec<Edition>,
    recording: Recording,
}

/// One package in a multi-package HIR fixture.
#[derive(Clone, Copy)]
pub(crate) struct PackageSpec {
    /// The package name exposed to symbols and importer aliases.
    pub(crate) name: &'static str,
    /// The package files, in order; the first file is the entry file.
    pub(crate) files: &'static [(&'static str, &'static str)],
    /// Modules exposed by the package API. Empty exposes the root.
    pub(crate) exposed: &'static [&'static str],
    /// Dependency aliases and the package index each names.
    pub(crate) dependencies: &'static [(&'static str, usize)],
}

impl Fixture {
    pub(crate) fn new(source: &str) -> Self {
        let map = Arc::new(SourceMap::new());
        let id = map
            .add_file(FileName::virtual_name("test.sl"), source.to_owned())
            .expect("test source registers");
        let file = map.file(id).expect("registered file exists");
        let recording = Recording::default();
        let dcx = DiagCtxt::new(Box::new(recording.clone()), Arc::clone(&map));
        Self {
            map,
            dcx,
            interner: Interner::new(),
            file,
            files: vec![id],
            package_files: Vec::new(),
            packages: Vec::new(),
            editions: Vec::new(),
            recording,
        }
    }

    /// Creates a fixture whose files are compiled as one package.
    ///
    /// Each entry is `(file name, source)`. The first file is the package
    /// entry point; module membership comes from each file's `module`
    /// declaration.
    pub(crate) fn package(files: &[(&str, &str)]) -> Self {
        assert!(!files.is_empty(), "a package needs at least one file");
        let map = Arc::new(SourceMap::new());
        let mut ids = Vec::with_capacity(files.len());
        for (name, source) in files {
            let id = map
                .add_file(FileName::virtual_name(*name), (*source).to_owned())
                .unwrap_or_else(|_| panic!("test source `{name}` registers"));
            ids.push(id);
        }
        let file = map.file(ids[0]).expect("registered file exists");
        let recording = Recording::default();
        let dcx = DiagCtxt::new(Box::new(recording.clone()), Arc::clone(&map));
        Self {
            map,
            dcx,
            interner: Interner::new(),
            file,
            files: ids.clone(),
            package_files: vec![ids],
            packages: Vec::new(),
            editions: Vec::new(),
            recording,
        }
    }

    /// Creates a fixture whose packages are compiled together in plan order.
    ///
    /// A package may depend only on a package that appears before it. The last
    /// package is the primary package, mirroring a dependency list compiled for
    /// its final consumer.
    pub(crate) fn from_packages(specs: &[PackageSpec]) -> Self {
        assert!(!specs.is_empty(), "a plan needs at least one package");
        let map = Arc::new(SourceMap::new());
        let mut package_files = Vec::with_capacity(specs.len());
        for spec in specs {
            assert!(
                !spec.files.is_empty(),
                "package `{}` needs at least one file",
                spec.name
            );
            let mut ids = Vec::with_capacity(spec.files.len());
            for (name, source) in spec.files {
                let id = map
                    .add_file(FileName::virtual_name(*name), (*source).to_owned())
                    .unwrap_or_else(|_| panic!("test source `{name}` registers"));
                ids.push(id);
            }
            package_files.push(ids);
        }
        let files: Vec<seal_span::FileId> = package_files.iter().flatten().copied().collect();
        let file = map.file(files[0]).expect("registered file exists");
        let recording = Recording::default();
        let dcx = DiagCtxt::new(Box::new(recording.clone()), Arc::clone(&map));
        Self {
            map,
            dcx,
            interner: Interner::new(),
            file,
            files,
            package_files,
            packages: specs.to_vec(),
            editions: Vec::new(),
            recording,
        }
    }

    /// Creates a multi-package fixture with one edition per package.
    pub(crate) fn from_packages_with_editions(specs: &[PackageSpec], editions: &[Edition]) -> Self {
        assert_eq!(
            specs.len(),
            editions.len(),
            "every package needs one edition"
        );
        let mut fixture = Self::from_packages(specs);
        fixture.editions = editions.to_vec();
        fixture
    }

    fn parse(&self) -> Ast {
        let lexed = seal_lexer::lex(&self.file, &self.dcx, &self.interner);
        seal_parser::parse(&lexed, &self.dcx, &self.interner)
    }

    pub(crate) fn lower(&self) -> Hir {
        let ast = self.parse();
        crate::lower(&ast, &self.dcx, &self.interner)
    }

    pub(crate) fn lower_with_c_abi(&self, c_abi: seal_ty::CAbiProperties) -> Hir {
        let ast = self.parse();
        crate::lower_with_c_abi(&ast, &self.dcx, &self.interner, c_abi)
    }

    /// Lowers every file in the fixture as one package.
    pub(crate) fn lower_package(&self) -> Hir {
        self.lower_package_exposing(&[])
    }

    /// Lowers the package with the given module paths in its public API.
    ///
    /// An empty list exposes the package root.
    pub(crate) fn lower_package_exposing(&self, exposed: &[&str]) -> Hir {
        self.lower_package_exposing_with_cfg(exposed, crate::CfgEnvironment::default())
    }

    /// Lowers every file in the fixture with explicit configuration facts.
    pub(crate) fn lower_package_with_cfg(&self, cfg: crate::CfgEnvironment) -> Hir {
        self.lower_package_exposing_with_cfg(&[], cfg)
    }

    fn lower_package_exposing_with_cfg(&self, exposed: &[&str], cfg: crate::CfgEnvironment) -> Hir {
        let mut asts = Vec::with_capacity(self.files.len());
        for id in &self.files {
            let file = self.map.file(*id).expect("registered file exists");
            let lexed = seal_lexer::lex(&file, &self.dcx, &self.interner);
            asts.push((*id, seal_parser::parse(&lexed, &self.dcx, &self.interner)));
        }
        let mut plan = crate::PackagePlan::single(self.interner.intern("test"), self.files[0]);
        if !exposed.is_empty() {
            plan.packages[0].exposed = exposed
                .iter()
                .map(|path| module_path(&self.interner, path))
                .collect();
        }
        plan.packages[0].files = self
            .files
            .iter()
            .enumerate()
            .map(|(index, file)| crate::PackagePlanFile {
                file: *file,
                primary: index == 0,
            })
            .collect();
        plan.packages[0].cfg = cfg;
        crate::lower_package(
            &plan,
            &asts,
            &self.dcx,
            &self.interner,
            seal_ty::CAbiProperties::default(),
        )
    }

    /// Lowers every package in plan order.
    pub(crate) fn lower_packages(&self) -> Hir {
        self.lower_packages_with(&[], &[])
    }

    /// Lowers every package with a prelude on `package` read from
    /// `source_package::module`.
    pub(crate) fn lower_packages_with_prelude(
        &self,
        package: usize,
        source_package: usize,
        module: &str,
    ) -> Hir {
        self.lower_packages_with_preludes_and_cfgs(
            &[(
                package,
                PreludeSource {
                    package: PackageId::from_usize(source_package),
                    module: module_path(&self.interner, module),
                },
            )],
            &[],
        )
    }

    /// Lowers every package with explicit preludes, keyed by package index.
    pub(crate) fn lower_packages_with_preludes(&self, preludes: &[(usize, PreludeSource)]) -> Hir {
        self.lower_packages_with_preludes_and_cfgs(preludes, &[])
    }

    /// Lowers every package with explicit preludes and `cfg` overrides.
    pub(crate) fn lower_packages_with_preludes_and_cfgs(
        &self,
        preludes: &[(usize, PreludeSource)],
        cfgs: &[(usize, crate::CfgEnvironment)],
    ) -> Hir {
        self.lower_packages_with(preludes, cfgs)
    }

    fn lower_packages_with(
        &self,
        preludes: &[(usize, PreludeSource)],
        cfgs: &[(usize, crate::CfgEnvironment)],
    ) -> Hir {
        let mut asts = Vec::with_capacity(self.files.len());
        for id in &self.files {
            let file = self.map.file(*id).expect("registered file exists");
            let lexed = seal_lexer::lex(&file, &self.dcx, &self.interner);
            asts.push((*id, seal_parser::parse(&lexed, &self.dcx, &self.interner)));
        }

        let mut packages = Vec::with_capacity(self.packages.len());
        for (index, spec) in self.packages.iter().enumerate() {
            let files = self.package_files[index]
                .iter()
                .enumerate()
                .map(|(position, file)| PackagePlanFile {
                    file: *file,
                    primary: position == 0,
                })
                .collect();
            let exposed = if spec.exposed.is_empty() {
                vec![ModulePath::root()]
            } else {
                spec.exposed
                    .iter()
                    .map(|path| module_path(&self.interner, path))
                    .collect()
            };
            let dependencies = spec
                .dependencies
                .iter()
                .map(|(alias, package)| PackagePlanDependency {
                    alias: self.interner.intern(alias),
                    package: PackageId::from_usize(*package),
                })
                .collect();
            packages.push(PackagePlanEntry {
                name: self.interner.intern(spec.name),
                edition: self
                    .editions
                    .get(index)
                    .copied()
                    .unwrap_or(Edition::CURRENT),
                instance: PackageInstanceId::from_usize(index),
                files,
                exposed,
                dependencies,
                cfg: cfgs
                    .iter()
                    .find(|(target, _)| *target == index)
                    .map_or_else(crate::CfgEnvironment::default, |(_, cfg)| cfg.clone()),
                prelude: preludes
                    .iter()
                    .find(|(target, _)| *target == index)
                    .map(|(_, source)| source.clone()),
            });
        }

        let primary = PackageId::from_usize(packages.len() - 1);
        let plan = PackagePlan { packages, primary };
        crate::lower_package(
            &plan,
            &asts,
            &self.dcx,
            &self.interner,
            seal_ty::CAbiProperties::default(),
        )
    }

    pub(crate) fn diagnostics(&self) -> Vec<Diagnostic> {
        self.recording.diagnostics()
    }

    pub(crate) fn codes(&self) -> Vec<String> {
        self.diagnostics()
            .iter()
            .map(|diagnostic| {
                diagnostic
                    .code()
                    .map_or_else(|| "<none>".to_owned(), |code| code.as_str().to_owned())
            })
            .collect()
    }

    pub(crate) fn assert_clean(&self) {
        let diagnostics = self.diagnostics();
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {:?}",
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.code(), diagnostic.message()))
                .collect::<Vec<_>>()
        );
    }

    pub(crate) fn expect(&self, code: &str, message_fragment: &str) {
        let diagnostics = self.diagnostics();
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.code().map(|code| code.as_str()) == Some(code)
                    && diagnostic.message().contains(message_fragment)
            }),
            "expected {code} containing {message_fragment:?}, got {:?}",
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.code(), diagnostic.message()))
                .collect::<Vec<_>>()
        );
    }

    pub(crate) fn symbol(&self, symbol: Symbol) -> String {
        self.interner.resolve(symbol).to_owned()
    }
}

/// Parses a module path written with `::`, or `root`.
fn module_path(interner: &Interner, path: &str) -> ModulePath {
    if path == "root" {
        return ModulePath::root();
    }
    ModulePath::new(
        path.split("::")
            .map(|segment| interner.intern(segment))
            .collect(),
    )
}

/// Returns the primary package's root module.
pub(crate) fn root_module(hir: &Hir) -> ModuleId {
    hir.package(hir.primary_package()).root
}

/// Returns the child module `name` of `parent`.
pub(crate) fn child_module(hir: &Hir, fixture: &Fixture, parent: ModuleId, name: &str) -> ModuleId {
    let symbol = fixture.interner.intern(name);
    hir.child_module(parent, symbol)
        .unwrap_or_else(|| panic!("no submodule named `{name}`"))
}

/// Returns the item of `kind` named `name` declared directly in `module`.
pub(crate) fn item_in_module(
    hir: &Hir,
    fixture: &Fixture,
    module: ModuleId,
    kind: DefKind,
    name: &str,
) -> DefId {
    hir.items_in_module(module)
        .into_iter()
        .find(|def| {
            hir.definitions().kind(*def) == kind
                && fixture.symbol(hir.definitions().name(*def)) == name
        })
        .unwrap_or_else(|| panic!("no {kind:?} named `{name}` in the module"))
}

pub(crate) fn lower_clean(source: &str) -> (Fixture, Hir) {
    let fixture = Fixture::new(source);
    let hir = fixture.lower();
    fixture.assert_clean();
    (fixture, hir)
}

/// Lowers a multi-file package and asserts that no diagnostic was reported.
pub(crate) fn lower_package_clean(files: &[(&str, &str)]) -> (Fixture, Hir) {
    let fixture = Fixture::package(files);
    let hir = fixture.lower_package();
    fixture.assert_clean();
    (fixture, hir)
}

/// Returns the function named `name` and its body.
pub(crate) fn function<'a>(fixture: &Fixture, hir: &'a Hir, name: &str) -> (DefId, &'a Body) {
    let definition = hir
        .definitions()
        .iter_enumerated()
        .find(|(_, data)| {
            data.kind == DefKind::Function
                && data.parent.is_none()
                && fixture.symbol(data.name) == name
        })
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no function named `{name}`"));
    let body = hir
        .body_of(definition)
        .unwrap_or_else(|| panic!("function `{name}` has no body"));
    (definition, body)
}

pub(crate) fn tail(hir: &Hir, body: &Body) -> HirExprId {
    hir.block(body.root)
        .tail
        .expect("block has a tail expression")
}

pub(crate) fn path_res(hir: &Hir, expr: HirExprId) -> Res {
    match hir.expr(expr).kind.clone() {
        ExprKind::Path { res, .. } => res,
        other => panic!("expected a path, got {other:?}"),
    }
}

pub(crate) fn expr_statement(hir: &Hir, statement: HirStmtId) -> HirExprId {
    match hir.stmt(statement).kind.clone() {
        StmtKind::Expr(expr) => expr,
        other => panic!("expected an expression statement, got {other:?}"),
    }
}

pub(crate) fn let_binding(hir: &Hir, statement: HirStmtId) -> DefId {
    match hir.stmt(statement).kind.clone() {
        StmtKind::Let { local, .. } => local,
        other => panic!("expected a let, got {other:?}"),
    }
}

/// Verifies that every ID resolves, every node span is inside the file, every
/// definition of a parameter or binding is owned by its function, and every
/// arena node is reachable from a body.
pub(crate) fn assert_invariants(fixture: &Fixture, hir: &Hir) {
    let mut reachability = Reachability {
        fixture,
        hir,
        owner: None,
        bodies: BTreeSet::new(),
        blocks: BTreeSet::new(),
        stmts: BTreeSet::new(),
        exprs: BTreeSet::new(),
    };
    reachability.run();
}

struct Reachability<'a> {
    fixture: &'a Fixture,
    hir: &'a Hir,
    owner: Option<DefId>,
    bodies: BTreeSet<BodyId>,
    blocks: BTreeSet<HirBlockId>,
    stmts: BTreeSet<HirStmtId>,
    exprs: BTreeSet<HirExprId>,
}

impl Reachability<'_> {
    fn run(&mut self) {
        for (body_id, body) in self.hir.bodies().iter_enumerated() {
            assert!(
                self.bodies.insert(body_id),
                "body {body_id:?} visited twice"
            );
            let owner = self.hir.definitions().get(body.owner);
            match body.kind {
                BodyKind::Function => assert!(
                    matches!(owner.kind, DefKind::Function | DefKind::TraitMethod),
                    "a function body must be owned by a function or a trait method"
                ),
                BodyKind::Const => {
                    assert_eq!(owner.kind, DefKind::Const, "a const body owns a constant");
                    assert!(body.params.is_empty(), "a const body has no parameters");
                }
                BodyKind::Static { mutable } => {
                    assert_eq!(owner.kind, DefKind::Static, "a static body owns a static");
                    assert_eq!(owner.mutable, mutable, "the body records static mutability");
                    assert!(body.params.is_empty(), "a static body has no parameters");
                }
                BodyKind::Closure => assert_eq!(
                    owner.kind,
                    DefKind::Closure,
                    "a closure body is owned by its closure"
                ),
            }
            assert_eq!(
                self.hir.body_id_of(body.owner),
                Some(body_id),
                "body_of must agree with the arena"
            );
            self.owner = Some(body.owner);
            self.assert_span(owner.span);
            self.assert_span(owner.name_span);
            if let Some(ret) = body.ret {
                self.check_type(ret);
            }
            for param in &body.params {
                let data = self.hir.definitions().get(param.def);
                assert_eq!(data.kind, DefKind::Param);
                assert_eq!(data.parent, Some(body.owner));
                assert!(!data.mutable, "parameters are immutable");
                self.assert_span(data.span);
                self.assert_span(data.name_span);
                self.check_type(param.ty);
            }
            self.check_block(body.root);
        }

        assert_eq!(self.bodies.len(), self.hir.bodies().len());
        assert_eq!(self.blocks.len(), self.hir.block_arena().len());
        assert_eq!(self.stmts.len(), self.hir.stmt_arena().len());
        assert_eq!(self.exprs.len(), self.hir.expr_arena().len());
    }

    fn assert_span(&self, span: Span) {
        assert!(
            self.fixture.file.contains_span(span),
            "span {span:?} is outside the file"
        );
    }

    fn check_type(&mut self, ty: TyId) {
        // Resolving the kind panics on an ID from another type context, which is
        // the invariant this check protects.
        match self.hir.ty_ctxt().kind(ty) {
            seal_ty::TyKind::Param(param) => {
                let data = self.hir.ty_ctxt().param_data(param);
                assert_eq!(
                    self.hir.definitions().get(data.def).kind,
                    DefKind::GenericParam,
                    "a type parameter must name a declaration"
                );
            }
            seal_ty::TyKind::Adt { args, .. } => self.check_substs(args),
            seal_ty::TyKind::Array { element, .. } | seal_ty::TyKind::Slice { element } => {
                self.check_type(element)
            }
            seal_ty::TyKind::Tuple(tuple) => {
                for element in self.hir.ty_ctxt().tuple_elements(tuple) {
                    self.check_type(element);
                }
            }
            seal_ty::TyKind::Ref { pointee, .. } => self.check_type(pointee),
            _ => {}
        }
    }

    fn check_substs(&mut self, substs: seal_ty::SubstsId) {
        for argument in self.hir.ty_ctxt().substs_args(substs) {
            self.check_type(argument);
        }
    }

    fn check_optional_substs(&mut self, substs: Option<seal_ty::SubstsId>) {
        if let Some(substs) = substs {
            self.check_substs(substs);
        }
    }

    fn check_block(&mut self, id: HirBlockId) {
        if !self.blocks.insert(id) {
            return;
        }
        let (span, statement_count, tail) = {
            let block = self.hir.block(id);
            (block.span, block.statements.len(), block.tail)
        };
        self.assert_span(span);
        for index in 0..statement_count {
            let statement = self.hir.block(id).statements[index];
            self.check_stmt(statement);
        }
        if let Some(tail) = tail {
            self.check_expr(tail);
        }
    }

    fn check_stmt(&mut self, id: HirStmtId) {
        if !self.stmts.insert(id) {
            return;
        }
        let (span, kind) = {
            let statement = self.hir.stmt(id);
            (statement.span, statement.kind.clone())
        };
        self.assert_span(span);
        match kind {
            StmtKind::Const { def } | StmtKind::Static { def } => {
                let data = self.hir.definitions().get(def);
                assert!(
                    matches!(data.kind, DefKind::Const | DefKind::Static),
                    "a declaration statement names a constant or static"
                );
                assert_eq!(
                    data.parent, self.owner,
                    "a local value belongs to its function"
                );
                assert!(
                    self.hir.body_of(def).is_some(),
                    "a local value owns its initializer body"
                );
                self.assert_span(data.name_span);
                self.assert_span(data.span);
            }
            StmtKind::Let { local, ty, init } => {
                let data = self.hir.definitions().get(local);
                assert_eq!(data.kind, DefKind::Binding);
                assert_eq!(data.parent, self.owner);
                self.assert_span(data.span);
                self.assert_span(data.name_span);
                if let Some(ty) = ty {
                    self.check_type(ty);
                }
                if let Some(init) = init {
                    self.check_expr(init);
                }
            }
            StmtKind::Return(value) => {
                if let Some(value) = value {
                    self.check_expr(value);
                }
            }
            StmtKind::While { condition, body } => {
                self.check_expr(condition);
                self.check_block(body);
            }
            StmtKind::WhileLet {
                pattern,
                scrutinee,
                body,
            } => {
                self.check_pattern(pattern);
                self.check_expr(scrutinee);
                self.check_block(body);
            }
            StmtKind::For {
                pattern,
                source,
                body,
            } => {
                self.check_pattern(pattern);
                match source {
                    crate::ForSource::Range { start, end, .. } => {
                        self.check_expr(start);
                        self.check_expr(end);
                    }
                    crate::ForSource::Expr(expr) => self.check_expr(expr),
                }
                self.check_block(body);
            }
            StmtKind::Expr(expr) => self.check_expr(expr),
            StmtKind::Break | StmtKind::Continue | StmtKind::Error => {}
        }
    }

    fn check_expr(&mut self, id: HirExprId) {
        if !self.exprs.insert(id) {
            return;
        }
        let (span, kind) = {
            let expr = self.hir.expr(id);
            (expr.span, expr.kind.clone())
        };
        self.assert_span(span);

        // A local name inside a closure body must belong to that closure. An
        // outer local that leaked through resolution would be an orphan path
        // instead of an `ExprKind::Capture`.
        if let ExprKind::Path {
            res: Res::Local(def),
            ..
        } = &kind
            && let Some(owner) = self.owner
            && self.hir.is_closure(owner)
        {
            assert_eq!(
                self.hir.definitions().get(*def).parent,
                Some(owner),
                "a closure body cannot name an outer local directly"
            );
        }

        if let Some(place) = self.hir.place(id) {
            let data = self.hir.definitions().get(place.local_def());
            match data.kind {
                DefKind::Binding | DefKind::Param => assert_eq!(data.parent, self.owner),
                DefKind::Capture => {
                    assert_eq!(data.parent, self.owner, "a capture belongs to its closure")
                }
                DefKind::Const | DefKind::Static => assert!(
                    data.parent.is_none() || data.parent == self.owner,
                    "a value root belongs to file scope or to this body's owner"
                ),
                other => {
                    panic!("a place must name a local, a constant, or a static, got {other:?}")
                }
            }
        }

        match kind {
            ExprKind::Unary { operand, .. }
            | ExprKind::Borrow { operand, .. }
            | ExprKind::Deref { operand } => self.check_expr(operand),
            ExprKind::OwnedView { owner, source } => {
                self.check_expr(owner);
                self.check_expr(source);
            }
            ExprKind::Field { base, .. } | ExprKind::TupleField { base, .. } => {
                self.check_expr(base)
            }
            ExprKind::Tuple { elements } => {
                for element in elements {
                    self.check_expr(element);
                }
            }
            ExprKind::Array { elements } => {
                for element in elements {
                    self.check_expr(element);
                }
            }
            ExprKind::ArrayRepeat { value, .. } => self.check_expr(value),
            ExprKind::Index { base, index } => {
                self.check_expr(base);
                self.check_expr(index);
            }
            ExprKind::Slice { base, start, end } => {
                self.check_expr(base);
                if let Some(start) = start {
                    self.check_expr(start);
                }
                if let Some(end) = end {
                    self.check_expr(end);
                }
            }
            ExprKind::SliceRef {
                base, start, end, ..
            } => {
                self.check_expr(base);
                if let Some(start) = start {
                    self.check_expr(start);
                }
                if let Some(end) = end {
                    self.check_expr(end);
                }
            }
            ExprKind::Len { operand }
            | ExprKind::Drop { operand }
            | ExprKind::Forget { operand } => self.check_expr(operand),
            ExprKind::Struct { adt, args, fields } => {
                let data = self.hir.definitions().get(adt);
                assert_eq!(data.kind, DefKind::Struct);
                self.assert_span(data.span);
                self.check_optional_substs(args);
                for (field, value) in fields {
                    let field_data = self.hir.definitions().get(field);
                    assert_eq!(field_data.kind, DefKind::Field);
                    assert_eq!(field_data.parent, Some(adt));
                    self.check_expr(value);
                }
            }
            ExprKind::Variant {
                adt,
                variant,
                args,
                fields,
            } => {
                let data = self.hir.definitions().get(adt);
                assert_eq!(data.kind, DefKind::Enum);
                self.check_optional_substs(args);
                let aggregate = self.hir.adt(adt).expect("an enum reserves an aggregate");
                let variant_def = self
                    .hir
                    .ty_ctxt()
                    .variant(aggregate, variant)
                    .expect("the variant index is registered")
                    .def;
                let variant_data = self.hir.definitions().get(variant_def);
                assert_eq!(variant_data.kind, DefKind::Variant);
                assert_eq!(variant_data.parent, Some(adt));
                for (field, value) in fields {
                    let field_data = self.hir.definitions().get(field);
                    assert_eq!(field_data.kind, DefKind::Field);
                    assert_eq!(field_data.parent, Some(variant_def));
                    self.check_expr(value);
                }
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                self.check_expr(lhs);
                self.check_expr(rhs);
            }
            ExprKind::Cast {
                operand,
                as_span,
                target_span,
                ..
            } => {
                self.assert_span(as_span);
                self.assert_span(target_span);
                self.check_expr(operand);
            }
            ExprKind::Assign { target, value, .. } => {
                self.check_expr(target);
                self.check_expr(value);
            }
            ExprKind::Update { target, .. } => self.check_expr(target),
            ExprKind::Call { callee, args } => {
                self.check_expr(callee);
                for arg in args {
                    self.check_expr(arg);
                }
            }
            ExprKind::MethodCall {
                receiver,
                name_span,
                generic_args,
                args,
                ..
            } => {
                self.assert_span(name_span);
                self.check_optional_substs(generic_args);
                self.check_expr(receiver);
                for arg in args {
                    self.check_expr(arg);
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                self.check_expr(scrutinee);
                for arm in arms {
                    self.check_pattern(arm.pattern);
                    if let Some(guard) = arm.guard {
                        self.check_expr(guard);
                    }
                    self.check_expr(arm.body);
                }
            }
            ExprKind::Block(block) | ExprKind::Unsafe { block } => self.check_block(block),
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.check_expr(condition);
                self.check_block(then_branch);
                if let Some(else_branch) = else_branch {
                    self.check_block(else_branch);
                }
            }
            ExprKind::IfLet {
                pattern,
                scrutinee,
                then_branch,
                else_branch,
            } => {
                self.check_pattern(pattern);
                self.check_expr(scrutinee);
                self.check_block(then_branch);
                if let Some(else_branch) = else_branch {
                    self.check_block(else_branch);
                }
            }
            ExprKind::Closure { closure, captures } => {
                let data = self.hir.definitions().get(closure);
                assert_eq!(data.kind, DefKind::Closure);
                let definition = self
                    .hir
                    .closure(closure)
                    .expect("a closure expression has closure data");
                assert_eq!(definition.captures, captures);
                assert!(self.hir.is_closure(closure));
                let adt = self
                    .hir
                    .adt(closure)
                    .expect("a closure reserves an aggregate");
                assert_eq!(definition.adt, adt);
                assert_eq!(self.hir.closure_for_adt(adt), Some(closure));
                let aggregate = self.hir.ty_ctxt().adt(adt);
                assert!(aggregate.is_closure());
                assert_eq!(aggregate.fields.len(), captures.len());
                assert_eq!(
                    aggregate.generic_count as usize,
                    self.hir.flattened_generics(closure).len(),
                    "the environment is generic over the enclosing parameters"
                );
                let body = self.hir.body_of(closure).expect("a closure owns a body");
                assert_eq!(body.kind, BodyKind::Closure);
                for (index, capture) in captures.iter().enumerate() {
                    let field = self.hir.definitions().get(capture.field);
                    assert_eq!(field.kind, DefKind::Field);
                    assert_eq!(field.parent, Some(closure));
                    let binding = self.hir.definitions().get(capture.binding);
                    assert_eq!(binding.kind, DefKind::Capture);
                    assert_eq!(binding.parent, Some(closure));
                    assert_eq!(aggregate.fields[index].def, capture.field);
                    match capture.source {
                        crate::CaptureSource::Local(origin) => {
                            assert_eq!(origin, capture.origin)
                        }
                        crate::CaptureSource::Outer(slot) => {
                            let parent = self
                                .hir
                                .definitions()
                                .get(closure)
                                .parent
                                .and_then(|parent| self.hir.closure(parent))
                                .expect("an outer capture comes from an enclosing closure");
                            assert_eq!(
                                parent.captures[slot as usize].origin, capture.origin,
                                "an outer slot holds the same origin"
                            );
                        }
                    }
                }
            }
            ExprKind::Capture { closure, index } => {
                assert_eq!(
                    self.owner,
                    Some(closure),
                    "a capture is used in its closure"
                );
                let definition = self
                    .hir
                    .closure(closure)
                    .expect("a capture names an existing closure");
                let capture = definition
                    .captures
                    .get(index as usize)
                    .expect("a capture index is in range");
                assert_eq!(
                    self.hir.place(id).map(|place| place.local_def()),
                    Some(capture.binding),
                    "a capture is a place rooted at its binding"
                );
            }
            ExprKind::Assert { condition, .. } => self.check_expr(condition),
            ExprKind::AssertEq { lhs, rhs, .. } => {
                self.check_expr(lhs);
                self.check_expr(rhs);
            }
            ExprKind::Panic { message } => self.check_expr(message),
            ExprKind::Exit { code } => self.check_expr(code),
            ExprKind::TypeOf { operand } => self.check_expr(operand),
            ExprKind::TypeName { .. } | ExprKind::SizeOf { .. } | ExprKind::AlignOf { .. } => {}
            ExprKind::Path {
                args,
                function_args,
                ..
            } => {
                self.check_optional_substs(args);
                self.check_optional_substs(function_args);
            }
            ExprKind::Lit(_) | ExprKind::Error => {}
        }
    }

    fn check_pattern(&mut self, id: HirPatternId) {
        let (span, kind) = {
            let pattern = self.hir.pattern(id);
            (pattern.span, pattern.kind.clone())
        };
        self.assert_span(span);
        match kind {
            PatternKind::Binding { def, pattern, .. } => {
                let data = self.hir.definitions().get(def);
                assert!(data.kind.is_local(), "a pattern binding names a local");
                assert_eq!(data.parent, self.owner);
                if let Some(pattern) = pattern {
                    self.check_pattern(pattern);
                }
            }
            PatternKind::Variant { fields, .. } => {
                for (_, field) in fields {
                    self.check_pattern(field);
                }
            }
            PatternKind::Struct { adt, fields } => {
                let data = self.hir.definitions().get(adt);
                assert_eq!(data.kind, DefKind::Struct);
                for (field, pattern) in fields {
                    let field_data = self.hir.definitions().get(field);
                    assert_eq!(field_data.kind, DefKind::Field);
                    assert_eq!(field_data.parent, Some(adt));
                    self.check_pattern(pattern);
                }
            }
            PatternKind::Tuple(elements) => {
                for element in elements {
                    self.check_pattern(element);
                }
            }
            PatternKind::Array { prefix, suffix, .. } => {
                for element in prefix.iter().chain(suffix.iter()) {
                    self.check_pattern(*element);
                }
            }
            PatternKind::Or(alternatives) => {
                for alternative in alternatives {
                    self.check_pattern(alternative);
                }
            }
            PatternKind::Wildcard
            | PatternKind::Literal(_)
            | PatternKind::Range { .. }
            | PatternKind::Error => {}
        }
    }
}
