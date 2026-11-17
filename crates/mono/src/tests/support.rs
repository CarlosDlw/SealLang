//! Fixtures for monomorphization tests.

use std::io;
use std::sync::{Arc, Mutex};

use seal_ast::Ast;
use seal_diagnostics::{DiagCtxt, Diagnostic, Emitter};
use seal_hir::Hir;
use seal_mir::Mir;
use seal_span::{FileName, SourceFile, SourceMap};
use seal_symbol::{Interner, Symbol};

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
    recording: Recording,
    #[allow(dead_code)]
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
            recording,
            map,
        }
    }

    fn parse(&self) -> Ast {
        let lexed = seal_lexer::lex(&self.file, &self.dcx, &self.interner);
        seal_parser::parse(&lexed, &self.dcx, &self.interner)
    }

    /// Runs the front end without collection, requiring no diagnostics.
    pub(crate) fn frontend(&self) -> (Hir, Mir) {
        let ast = self.parse();
        let hir = seal_hir::lower(&ast, &self.dcx, &self.interner);
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
        let ast = self.parse();
        let hir = seal_hir::lower(&ast, &self.dcx, &self.interner);
        let typeck = seal_typeck::check(&hir, &self.dcx, &self.interner, 64);
        self.assert_clean();
        let mir = seal_mir::lower(&hir, &typeck);
        mir.validate_with(&hir)
            .expect("MIR lowering maintains its invariants");
        let program = collect_with(&hir, &mir, &self.dcx, &self.interner, limits);
        (hir, mir, program)
    }

    pub(crate) fn interner(&self) -> &Interner {
        &self.interner
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
