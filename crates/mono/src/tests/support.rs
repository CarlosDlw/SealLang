//! Fixtures for monomorphization tests.

use std::io;
use std::sync::{Arc, Mutex};

use seal_ast::Ast;
use seal_def::{DefId, DefKind};
use seal_diagnostics::{DiagCtxt, Diagnostic, Emitter};
use seal_hir::Hir;
use seal_mir::Mir;
use seal_span::{FileName, SourceFile, SourceMap};
use seal_symbol::{Interner, Symbol};
use seal_ty::AdtId;

use crate::MonoProgram;
use crate::collector::{Limits, collect_with};

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
    dcx: DiagCtxt,
    interner: Interner,
    file: Arc<SourceFile>,
    files: Vec<seal_span::FileId>,
    recording: Recording,
    map: Arc<SourceMap>,
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
            dcx,
            interner: Interner::new(),
            file,
            files: vec![id],
            recording,
            map,
        }
    }

    /// Creates a fixture whose files are compiled as one package.
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
            dcx,
            interner: Interner::new(),
            file,
            files: ids,
            recording,
            map,
        }
    }

    fn parse(&self) -> Ast {
        let lexed = seal_lexer::lex(&self.file, &self.dcx, &self.interner);
        seal_parser::parse(&lexed, &self.dcx, &self.interner)
    }

    /// Lowers the fixture to HIR, as a package when it has several files.
    fn lower_hir(&self) -> Hir {
        if self.files.len() == 1 {
            let ast = self.parse();
            return seal_hir::lower(&ast, &self.dcx, &self.interner);
        }
        let mut asts = Vec::with_capacity(self.files.len());
        for id in &self.files {
            let file = self.map.file(*id).expect("registered file exists");
            let lexed = seal_lexer::lex(&file, &self.dcx, &self.interner);
            asts.push((*id, seal_parser::parse(&lexed, &self.dcx, &self.interner)));
        }
        let mut plan = seal_hir::PackagePlan::single(self.interner.intern("test"), self.files[0]);
        plan.packages[0].files = self
            .files
            .iter()
            .enumerate()
            .map(|(index, file)| seal_hir::PackagePlanFile {
                file: *file,
                primary: index == 0,
            })
            .collect();
        seal_hir::lower_package(
            &plan,
            &asts,
            &self.dcx,
            &self.interner,
            seal_ty::CAbiProperties::default(),
        )
    }

    /// Lowers to HIR while retaining diagnostics for negative tests.
    pub(crate) fn hir_with_errors(&self) -> Hir {
        self.lower_hir()
    }

    /// Runs the front end without collection, requiring no diagnostics.
    pub(crate) fn frontend(&self) -> (Hir, Mir) {
        let hir = self.lower_hir();
        let typeck = seal_typeck::check(&hir, &self.dcx, &self.interner, 64);
        self.assert_clean();
        let mir = seal_mir::lower(&hir, &typeck);
        mir.validate_with(&hir)
            .expect("MIR lowering maintains its invariants");
        (hir, mir)
    }

    /// Runs the front end and collection with production limits.
    pub(crate) fn lower(&self) -> (Hir, Mir, MonoProgram) {
        self.lower_with(Limits::DEFAULT)
    }

    /// Runs the front end and collection with explicit limits.
    pub(crate) fn lower_with(&self, limits: Limits) -> (Hir, Mir, MonoProgram) {
        let (hir, mir, _constants, program) = self.collect(limits, &[]);
        (hir, mir, program)
    }

    /// Runs the front end and collection, returning the evaluated constants.
    pub(crate) fn lower_constants_with(
        &self,
        limits: Limits,
    ) -> (Hir, Mir, seal_const_eval::ConstProgram, MonoProgram) {
        self.collect(limits, &[])
    }

    /// Collects after registering a destructor for each named aggregate.
    ///
    /// `impl Drop` is not part of the surface yet, so a test that needs a type
    /// with a destructor registers one before MIR lowering, which is exactly
    /// where HIR lowering will record it once the syntax exists. Each pair is
    /// the aggregate and the function standing in for its destructor.
    pub(crate) fn lower_with_destructors(
        &self,
        destructors: &[(&str, &str)],
    ) -> (Hir, Mir, MonoProgram) {
        let (hir, mir, _constants, program) = self.collect(Limits::DEFAULT, destructors);
        (hir, mir, program)
    }

    /// Collects with explicit limits after registering destructors.
    pub(crate) fn lower_with_limits_and_destructors(
        &self,
        limits: Limits,
        destructors: &[(&str, &str)],
    ) -> (Hir, Mir, MonoProgram) {
        let (hir, mir, _constants, program) = self.collect(limits, destructors);
        (hir, mir, program)
    }

    /// Runs the front end, elaboration, constant evaluation, and collection.
    fn collect(
        &self,
        limits: Limits,
        destructors: &[(&str, &str)],
    ) -> (Hir, Mir, seal_const_eval::ConstProgram, MonoProgram) {
        let hir = self.lower_hir();
        for (aggregate, destructor) in destructors {
            let adt = self.aggregate(&hir, aggregate);
            let method = self.function(&hir, destructor);
            assert_eq!(
                hir.ty_ctxt().set_drop_method(adt, method),
                None,
                "`{aggregate}` starts without a destructor"
            );
        }
        let typeck = seal_typeck::check(&hir, &self.dcx, &self.interner, 64);
        self.assert_clean();
        let mut mir = seal_mir::lower(&hir, &typeck);
        mir.validate_with(&hir)
            .expect("MIR lowering maintains its invariants");
        seal_mir::elaborate_drops(&hir, &mut mir)
            .unwrap_or_else(|error| panic!("drop elaboration failed: {error}"));
        mir.validate_with(&hir)
            .expect("drop elaboration maintains the MIR invariants");
        let constants = seal_const_eval::evaluate(&hir, &mir, &self.dcx, &self.interner, 64);
        let program = collect_with(&hir, &mir, &constants, &self.dcx, &self.interner, limits);
        (hir, mir, constants, program)
    }

    /// Returns the aggregate declared with `name`.
    fn aggregate(&self, hir: &Hir, name: &str) -> AdtId {
        let definition = hir
            .definitions()
            .iter_enumerated()
            .find(|(_, data)| {
                matches!(data.kind, DefKind::Struct | DefKind::Enum)
                    && self.symbol(data.name) == *name
            })
            .map(|(id, _)| id)
            .unwrap_or_else(|| panic!("no aggregate named `{name}`"));
        hir.adt(definition)
            .unwrap_or_else(|| panic!("`{name}` has no aggregate id"))
    }

    /// Returns the function declared with `name`.
    fn function(&self, hir: &Hir, name: &str) -> DefId {
        hir.definitions()
            .iter_enumerated()
            .find(|(_, data)| data.kind == DefKind::Function && self.symbol(data.name) == *name)
            .map(|(id, _)| id)
            .unwrap_or_else(|| panic!("no function named `{name}`"))
    }

    pub(crate) fn interner(&self) -> &Interner {
        &self.interner
    }

    pub(crate) fn dcx(&self) -> &DiagCtxt {
        &self.dcx
    }

    pub(crate) fn symbol(&self, symbol: Symbol) -> String {
        self.interner.resolve(symbol).to_owned()
    }

    pub(crate) fn diagnostics(&self) -> Vec<Diagnostic> {
        self.recording.diagnostics()
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

    pub(crate) fn expect(&self, code: &str, message_fragment: &str) -> Diagnostic {
        let diagnostics = self.diagnostics();
        diagnostics
            .iter()
            .find(|diagnostic| {
                diagnostic.code().map(|code| code.as_str()) == Some(code)
                    && diagnostic.message().contains(message_fragment)
            })
            .cloned()
            .unwrap_or_else(|| {
                panic!(
                    "expected {code} containing {message_fragment:?}, got {:?}",
                    diagnostics
                        .iter()
                        .map(|diagnostic| (diagnostic.code(), diagnostic.message()))
                        .collect::<Vec<_>>()
                )
            })
    }
}

/// Collects `source`, requiring no front-end diagnostics.
pub(crate) fn collect_clean(source: &str) -> (Fixture, Hir, Mir, MonoProgram) {
    let fixture = Fixture::new(source);
    let (hir, mir, program) = fixture.lower();
    fixture.assert_clean();
    (fixture, hir, mir, program)
}

/// Collects a multi-file package, requiring no front-end diagnostics.
pub(crate) fn collect_clean_package(files: &[(&str, &str)]) -> (Fixture, Hir, Mir, MonoProgram) {
    let fixture = Fixture::package(files);
    let (hir, mir, program) = fixture.lower();
    fixture.assert_clean();
    (fixture, hir, mir, program)
}

/// Collects `source` and returns the evaluated constants too.
pub(crate) fn collect_clean_constants(
    source: &str,
) -> (
    Fixture,
    Hir,
    Mir,
    seal_const_eval::ConstProgram,
    MonoProgram,
) {
    let fixture = Fixture::new(source);
    let (hir, mir, constants, program) = fixture.lower_constants_with(Limits::DEFAULT);
    fixture.assert_clean();
    (fixture, hir, mir, constants, program)
}

/// Collects `source` and returns the fixture for diagnostics.
pub(crate) fn collect_errors(source: &str) -> (Fixture, Hir, Mir, MonoProgram) {
    let fixture = Fixture::new(source);
    let (hir, mir, program) = fixture.lower();
    (fixture, hir, mir, program)
}

/// Returns the rendered name of every collected instance.
pub(crate) fn names(fixture: &Fixture, hir: &Hir, program: &MonoProgram) -> Vec<String> {
    program
        .instances()
        .iter()
        .map(|instance| crate::instance_name(hir, &fixture.interner, instance.instance))
        .collect()
}
