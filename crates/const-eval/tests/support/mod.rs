//! Fixtures for const-evaluation tests.

#![allow(dead_code)]

use std::io;
use std::sync::{Arc, Mutex};

use seal_diagnostics::{DiagCtxt, Diagnostic, Emitter};
use seal_hir::Hir;
use seal_mir::Mir;
use seal_span::{FileName, SourceFile, SourceMap};
use seal_symbol::{Interner, Symbol};

use seal_const_eval::{ConstProgram, EvalLimits, evaluate, evaluate_with_limits};

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

pub(crate) struct Fixture {
    #[allow(dead_code)]
    map: Arc<SourceMap>,
    #[allow(dead_code)]
    file: Arc<SourceFile>,
    pub(crate) interner: Interner,
    recording: Recording,
    pub(crate) hir: Hir,
    #[allow(dead_code)]
    pub(crate) mir: Mir,
    pub(crate) program: ConstProgram,
}

impl Fixture {
    pub(crate) fn compile(source: &str) -> Self {
        Self::compile_with(source, EvalLimits::default())
    }

    pub(crate) fn compile_with(source: &str, limits: EvalLimits) -> Self {
        Self::compile_edited_with(source, limits, |_, _| {})
    }

    /// Compiles `source`, lets `edit` rewrite the MIR, then evaluates it.
    ///
    /// Evaluation runs after the edit, so a test can build a body shape that
    /// no source program produces yet.
    pub(crate) fn compile_edited(source: &str, edit: impl FnOnce(&Hir, &mut Mir)) -> Self {
        Self::compile_edited_with(source, EvalLimits::default(), edit)
    }

    pub(crate) fn compile_edited_with(
        source: &str,
        limits: EvalLimits,
        edit: impl FnOnce(&Hir, &mut Mir),
    ) -> Self {
        let map = Arc::new(SourceMap::new());
        let id = map
            .add_file(FileName::virtual_name("test.sl"), source.to_owned())
            .expect("test source registers");
        let file = map.file(id).expect("registered file exists");
        let recording = Recording::default();
        let dcx = DiagCtxt::new(Box::new(recording.clone()), Arc::clone(&map));
        let interner = Interner::new();
        let lexed = seal_lexer::lex(&file, &dcx, &interner);
        let ast = seal_parser::parse(&lexed, &dcx, &interner);
        let hir = seal_hir::lower(&ast, &dcx, &interner);
        let typeck = seal_typeck::check(&hir, &dcx, &interner, 64);
        let mut mir = seal_mir::lower(&hir, &typeck);
        // The driver elaborates before evaluating, and so does this fixture:
        // a dead drop site inside an initializer must be gone before constant
        // evaluation can reject it.
        seal_mir::elaborate_drops(&hir, &mut mir)
            .expect("drop elaboration preserves its invariants");
        edit(&hir, &mut mir);
        let program = evaluate_with_limits(&hir, &mir, &dcx, &interner, 64, limits);
        Self {
            map,
            file,
            interner,
            recording,
            hir,
            mir,
            program,
        }
    }

    pub(crate) fn symbol(&self, symbol: Symbol) -> String {
        self.interner.resolve(symbol).to_owned()
    }

    pub(crate) fn diagnostics(&self) -> Vec<Diagnostic> {
        self.recording
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub(crate) fn codes(&self) -> Vec<String> {
        self.diagnostics()
            .into_iter()
            .filter_map(|diagnostic| diagnostic.code().map(|code| code.as_str().to_owned()))
            .collect()
    }

    pub(crate) fn expect_code(&self, code: &str) {
        let codes = self.codes();
        assert!(
            codes.iter().any(|candidate| candidate == code),
            "expected {code}, got {codes:?} with messages {:?}",
            self.diagnostics()
                .iter()
                .map(Diagnostic::message)
                .collect::<Vec<_>>()
        );
    }

    pub(crate) fn definition(&self, name: &str) -> seal_def::DefId {
        self.hir
            .definitions()
            .iter_enumerated()
            .find(|(_, data)| self.symbol(data.name) == name)
            .map(|(def, _)| def)
            .unwrap_or_else(|| panic!("no definition named `{name}`"))
    }

    pub(crate) fn value(&self, name: &str) -> &seal_const_eval::ConstValue {
        let def = self.definition(name);
        self.program
            .value(def)
            .unwrap_or_else(|| panic!("`{name}` was not evaluated"))
    }
}

/// Compiles a source file and asserts that evaluation produced no errors.
pub(crate) fn clean(source: &str) -> Fixture {
    let fixture = Fixture::compile(source);
    let errors: Vec<_> = fixture
        .diagnostics()
        .into_iter()
        .filter(|diagnostic| diagnostic.level() == seal_diagnostics::Level::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "unexpected errors: {:?}",
        errors
            .iter()
            .map(|diagnostic| (diagnostic.code(), diagnostic.message()))
            .collect::<Vec<_>>()
    );
    fixture
}

#[allow(dead_code)]
pub(crate) fn evaluate_clean(source: &str) -> ConstProgram {
    let _ = evaluate;
    clean(source).program
}
