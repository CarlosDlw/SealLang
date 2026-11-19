//! Fixtures for borrow-checking tests.

use std::io;
use std::sync::{Arc, Mutex};

use seal_ast::Ast;
use seal_def::DefId;
use seal_diagnostics::{DiagCtxt, Diagnostic, Emitter};
use seal_hir::Hir;
use seal_mir::{Body, Mir};
use seal_span::{FileName, SourceFile, SourceMap};
use seal_symbol::{Interner, Symbol};

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
    #[allow(dead_code)]
    map: Arc<SourceMap>,
    dcx: DiagCtxt,
    interner: Interner,
    file: Arc<SourceFile>,
    recording: Recording,
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
            recording,
        }
    }

    fn parse(&self) -> Ast {
        let lexed = seal_lexer::lex(&self.file, &self.dcx, &self.interner);
        seal_parser::parse(&lexed, &self.dcx, &self.interner)
    }

    /// Runs lexing through MIR lowering, requiring a clean front end.
    pub(crate) fn lower(&self) -> (Hir, Mir) {
        self.lower_with_destructors(&[])
    }

    /// Lowers `source`, registering a destructor for each named aggregate.
    ///
    /// `impl Drop` is not part of the surface yet, so a test that needs a type
    /// with a destructor registers one before MIR lowering. The method points at
    /// the first function of the source, because these tests only need the
    /// registry to answer that the type implements `Drop`.
    pub(crate) fn lower_with_destructors(&self, names: &[&str]) -> (Hir, Mir) {
        let ast = self.parse();
        let hir = seal_hir::lower(&ast, &self.dcx, &self.interner);
        for name in names {
            let definition = hir
                .definitions()
                .iter_enumerated()
                .find(|(_, data)| {
                    matches!(
                        data.kind,
                        seal_def::DefKind::Struct | seal_def::DefKind::Enum
                    ) && self.symbol(data.name) == *name
                })
                .map(|(id, _)| id)
                .unwrap_or_else(|| panic!("no aggregate named `{name}`"));
            let adt = hir
                .adt(definition)
                .expect("an aggregate definition has an aggregate id");
            let method = hir
                .definitions()
                .iter_enumerated()
                .find(|(_, data)| data.kind == seal_def::DefKind::Function)
                .map(|(id, _)| id)
                .expect("the source declares a function to stand in for the destructor");
            assert_eq!(
                hir.ty_ctxt().set_drop_method(adt, method),
                None,
                "the aggregate starts without a destructor"
            );
        }
        let typeck = seal_typeck::check(&hir, &self.dcx, &self.interner, 64);
        self.assert_clean();
        let mir = seal_mir::lower(&hir, &typeck);
        mir.validate_with(&hir)
            .expect("MIR lowering maintains its invariants");
        (hir, mir)
    }

    /// Runs the pipeline through borrow checking, with the named aggregates
    /// implementing `Drop`.
    pub(crate) fn check_with_destructors(&self, names: &[&str]) {
        let (hir, mir) = self.lower_with_destructors(names);
        let program = seal_const_eval::evaluate(&hir, &mir, &self.dcx, &self.interner, 64);
        crate::check(&hir, &mir, Some(&program), &self.dcx, &self.interner);
    }

    pub(crate) fn symbol(&self, symbol: Symbol) -> String {
        self.interner.resolve(symbol).to_owned()
    }

    /// Runs the whole pipeline through borrow checking.
    pub(crate) fn check(&self) {
        let (hir, mir) = self.lower();
        let program = seal_const_eval::evaluate(&hir, &mir, &self.dcx, &self.interner, 64);
        crate::check(&hir, &mir, Some(&program), &self.dcx, &self.interner);
    }

    /// Runs borrow checking and returns the `noalias` facts for the program.
    pub(crate) fn check_facts(&self) -> (Hir, Mir, crate::AliasingFacts) {
        let (hir, mir) = self.lower();
        let program = seal_const_eval::evaluate(&hir, &mir, &self.dcx, &self.interner, 64);
        let facts = crate::check(&hir, &mir, Some(&program), &self.dcx, &self.interner);
        self.assert_clean();
        (hir, mir, facts)
    }

    /// Runs borrow checking on a body built by a test.
    pub(crate) fn check_body(&self, hir: &Hir, body: &Body) {
        crate::check_body(hir, body, &self.dcx, &self.interner);
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
}

/// Runs the pipeline and requires that borrow checking reports nothing.
pub(crate) fn check_clean(source: &str) -> Fixture {
    let fixture = Fixture::new(source);
    fixture.check();
    fixture.assert_clean();
    fixture
}

/// Runs the pipeline and returns the fixture for inspecting diagnostics.
pub(crate) fn check_errors(source: &str) -> Fixture {
    let fixture = Fixture::new(source);
    fixture.check();
    fixture
}

/// Returns the owner of the first body, for tests that build their own body.
pub(crate) fn main_owner(hir: &Hir) -> DefId {
    hir.bodies()
        .iter()
        .next()
        .expect("the source declares at least one body")
        .owner
}
