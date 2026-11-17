//! Fixtures and structural checks for type-checking tests.

use std::collections::BTreeSet;
use std::io;
use std::sync::{Arc, Mutex};

use seal_ast::Ast;
use seal_def::{DefId, DefKind};
use seal_diagnostics::{DiagCtxt, Diagnostic, Emitter};
use seal_hir::{Body, ExprKind, Hir, HirExprId, HirStmtId, Res, StmtKind};
use seal_span::{FileName, SourceFile, SourceMap};
use seal_symbol::{Interner, Symbol};
use seal_ty::TyId;

use crate::TypeckResults;

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

    pub(crate) fn check(&self, pointer_width: u8) -> (Hir, TypeckResults) {
        let ast = self.parse();
        let hir = seal_hir::lower(&ast, &self.dcx, &self.interner);
        let results = crate::check(&hir, &self.dcx, &self.interner, pointer_width);
        (hir, results)
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

    pub(crate) fn interner(&self) -> &Interner {
        &self.interner
    }
}

pub(crate) fn check_clean(source: &str) -> (Fixture, Hir, TypeckResults) {
    let fixture = Fixture::new(source);
    let (hir, results) = fixture.check(64);
    fixture.assert_clean();
    assert_invariants(&fixture, &hir, &results);
    (fixture, hir, results)
}

pub(crate) fn check_with_width(source: &str, pointer_width: u8) -> (Fixture, Hir, TypeckResults) {
    let fixture = Fixture::new(source);
    let (hir, results) = fixture.check(pointer_width);
    assert_invariants(&fixture, &hir, &results);
    (fixture, hir, results)
}

pub(crate) fn check_errors(source: &str) -> (Fixture, Hir, TypeckResults) {
    let fixture = Fixture::new(source);
    let (hir, results) = fixture.check(64);
    assert_invariants(&fixture, &hir, &results);
    (fixture, hir, results)
}

/// Returns the function named `name` and its body.
pub(crate) fn function<'a>(fixture: &Fixture, hir: &'a Hir, name: &str) -> (DefId, &'a Body) {
    let definition = hir
        .definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == DefKind::Function && fixture.symbol(data.name) == name)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no function named `{name}`"));
    let body = hir
        .body_of(definition)
        .unwrap_or_else(|| panic!("function `{name}` has no body"));
    (definition, body)
}

pub(crate) fn statements<'a>(hir: &'a Hir, body: &Body) -> &'a [HirStmtId] {
    &hir.block(body.root).statements
}

pub(crate) fn tail(hir: &Hir, body: &Body) -> HirExprId {
    hir.block(body.root)
        .tail
        .expect("block has a tail expression")
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

pub(crate) fn expr_ty(results: &TypeckResults, expr: HirExprId) -> TyId {
    results
        .expr_ty(expr)
        .unwrap_or_else(|| panic!("expression {expr:?} has no type"))
}

pub(crate) fn local_ty(results: &TypeckResults, local: DefId) -> TyId {
    results
        .local_ty(local)
        .unwrap_or_else(|| panic!("local {local:?} has no type"))
}

pub(crate) fn ty_text(hir: &Hir, ty: TyId) -> String {
    hir.ty_ctxt().display(ty).to_string()
}

/// Renders a type with struct names resolved through the fixture interner.
pub(crate) fn ty_text_with(fixture: &Fixture, hir: &Hir, ty: TyId) -> String {
    hir.ty_ctxt()
        .display_with(ty, fixture.interner())
        .to_string()
}

pub(crate) fn expect_ty(hir: &Hir, ty: TyId, expected: &str) {
    assert_eq!(
        ty_text(hir, ty),
        expected,
        "expected type `{expected}`, found `{}`",
        ty_text(hir, ty)
    );
}

/// Checks that the checker produced a type for every value expression and every
/// local, and a signature for every function.
pub(crate) fn assert_invariants(fixture: &Fixture, hir: &Hir, results: &TypeckResults) {
    let mut callees = BTreeSet::new();
    for expr in hir.expr_arena().iter() {
        if let ExprKind::Call { callee, .. } = &expr.kind
            && matches!(
                &hir.expr(*callee).kind,
                ExprKind::Path {
                    res: Res::Function(_),
                    ..
                }
            )
        {
            callees.insert(*callee);
        }
    }
    for (id, _) in hir.expr_arena().iter_enumerated() {
        if !callees.contains(&id) {
            assert!(
                results.expr_ty(id).is_some(),
                "expression {id:?} has no type"
            );
        }
    }

    for (id, data) in hir.definitions().iter_enumerated() {
        if data.kind.is_local() {
            assert!(
                results.local_ty(id).is_some(),
                "{} `{}` has no type",
                data.kind.description(),
                fixture.symbol(data.name)
            );
        }
    }

    for body in hir.bodies().iter() {
        let signature = results.signature(body.owner).unwrap_or_else(|| {
            panic!(
                "function `{}` has no signature",
                fixture.symbol(hir.definitions().name(body.owner))
            )
        });
        assert_eq!(
            signature.params.len(),
            body.params.len(),
            "signature and body disagree on parameters"
        );
        let _ = hir.ty_ctxt().kind(signature.ret);
        for ty in &signature.params {
            let _ = hir.ty_ctxt().kind(*ty);
        }
    }
}
