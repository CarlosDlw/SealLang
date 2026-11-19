//! Fixtures and structural helpers for MIR tests.

use std::io;
use std::sync::{Arc, Mutex};

use seal_ast::Ast;
use seal_def::DefKind;
use seal_diagnostics::{DiagCtxt, Diagnostic, Emitter};
use seal_hir::Hir;
use seal_span::{FileName, SourceFile, SourceMap};
use seal_symbol::{Interner, Symbol};
use seal_ty::TyId;

use crate::body::Body;
use crate::id::LocalId;
use crate::mir::Mir;
use crate::place::Place;
use crate::rvalue::Rvalue;
use crate::statement::Statement;

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

    fn lower(&self) -> (Hir, Mir) {
        let ast = self.parse();
        let hir = seal_hir::lower(&ast, &self.dcx, &self.interner);
        let results = seal_typeck::check(&hir, &self.dcx, &self.interner, 64);
        let mir = crate::lower(&hir, &results);
        (hir, mir)
    }

    pub(crate) fn symbol(&self, symbol: Symbol) -> String {
        self.interner.resolve(symbol).to_owned()
    }

    /// Displays `ty` with the test interner, so parameters show their names.
    pub(crate) fn ty_text(&self, hir: &Hir, ty: TyId) -> String {
        hir.ty_ctxt().display_with(ty, &self.interner).to_string()
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
}

/// Lowers `source` end to end, requiring a clean check and a valid MIR tree.
pub(crate) fn lower_clean(source: &str) -> (Fixture, Hir, Mir) {
    let fixture = Fixture::new(source);
    let (hir, mir) = fixture.lower();
    fixture.assert_clean();
    mir.validate_with(&hir)
        .expect("MIR lowering maintains its invariants");
    assert_eq!(
        mir.bodies().len(),
        hir.bodies().len(),
        "every HIR body must lower to exactly one MIR body"
    );
    for body in hir.bodies().iter() {
        let lowered = mir
            .body_of(body.owner)
            .unwrap_or_else(|| panic!("body for {:?} is missing", body.owner));
        assert_eq!(lowered.owner, body.owner);
        assert_eq!(lowered.arg_count, body.params.len());
    }
    (fixture, hir, mir)
}

/// Returns the definition of the `kind` item named `name`.
pub(crate) fn definition(
    fixture: &Fixture,
    hir: &Hir,
    kind: seal_def::DefKind,
    name: &str,
) -> seal_def::DefId {
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == kind && fixture.symbol(data.name) == name)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no {kind:?} named `{name}`"))
}

/// Returns the lowered body of the function named `name`.
pub(crate) fn function<'a>(fixture: &Fixture, hir: &Hir, mir: &'a Mir, name: &str) -> &'a Body {
    let owner = hir
        .definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == DefKind::Function && fixture.symbol(data.name) == name)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no function named `{name}`"));
    mir.body_of(owner)
        .unwrap_or_else(|| panic!("function `{name}` has no MIR body"))
}

/// Returns the local declared with `name`.
pub(crate) fn local_named(fixture: &Fixture, body: &Body, name: &str) -> LocalId {
    body.locals
        .iter_enumerated()
        .find(|(_, declaration)| {
            declaration
                .name
                .is_some_and(|symbol| fixture.symbol(symbol) == name)
        })
        .map(|(local, _)| local)
        .unwrap_or_else(|| panic!("no local named `{name}`"))
}

/// Returns the place and rvalue of an assignment statement.
///
/// # Panics
///
/// Panics when the statement is not an assignment, so a structural test fails
/// with the statement it actually found instead of silently checking nothing.
pub(crate) fn assignment(statement: &Statement) -> (&Place, &Rvalue) {
    match &statement.kind {
        crate::statement::StatementKind::Assign { place, rvalue } => (place, rvalue),
        other => panic!("expected an assignment, found {other:?}"),
    }
}

/// Returns the display form of a type.
pub(crate) fn ty_text(hir: &Hir, ty: TyId) -> String {
    hir.ty_ctxt().display(ty).to_string()
}
