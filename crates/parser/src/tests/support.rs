//! Fixtures and structural checks for parser tests.

use std::collections::BTreeSet;
use std::io;
use std::sync::{Arc, Mutex};

use seal_ast::{
    Ast, Block, BlockId, Expr, ExprId, ExprKind, FnDecl, ItemKind, Param, ParamId, Stmt, StmtId,
    StmtKind, Type, TypeId, TypeKind,
};
use seal_diagnostics::{DiagCtxt, Diagnostic, Emitter};
use seal_span::{FileName, SourceFile, SourceMap, Span};
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
    pub(crate) map: Arc<SourceMap>,
    pub(crate) dcx: DiagCtxt,
    pub(crate) interner: Interner,
    pub(crate) file: Arc<SourceFile>,
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

    pub(crate) fn parse(&self) -> Ast {
        let lexed = seal_lexer::lex(&self.file, &self.dcx, &self.interner);
        crate::parse(&lexed, &self.dcx, &self.interner)
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

    pub(crate) fn text(&self, span: Span) -> &str {
        self.file.snippet(span).expect("span covers this file")
    }
}

pub(crate) fn sole_function(ast: &Ast) -> &FnDecl {
    assert_eq!(
        ast.program().len(),
        1,
        "expected exactly one item, got {:?}",
        ast.program().items()
    );
    function(ast, 0)
}

pub(crate) fn function(ast: &Ast, index: usize) -> &FnDecl {
    let id = ast.program().items()[index];
    match &ast.item(id).kind {
        ItemKind::Func(declaration) => declaration,
        other => panic!("item {index} is not a function: {other:?}"),
    }
}

pub(crate) fn body<'a>(ast: &'a Ast, declaration: &FnDecl) -> &'a Block {
    ast.block(declaration.body)
}

pub(crate) fn tail<'a>(ast: &'a Ast, declaration: &FnDecl) -> &'a Expr {
    let block = body(ast, declaration);
    ast.expr(block.tail.expect("block has a tail expression"))
}

pub(crate) fn parse_clean(source: &str) -> (Fixture, Ast) {
    let fixture = Fixture::new(source);
    let ast = fixture.parse();
    fixture.assert_clean();
    (fixture, ast)
}

/// Verifies that every ID reference resolves, every node span is inside the
/// file, and every arena node is reachable from the program.
pub(crate) fn assert_invariants(fixture: &Fixture, ast: &Ast) {
    let mut exprs = BTreeSet::new();
    let mut stmts = BTreeSet::new();
    let mut blocks = BTreeSet::new();
    let mut types = BTreeSet::new();
    let mut params = BTreeSet::new();

    assert_eq!(
        ast.item_arena().len(),
        ast.program().len(),
        "every item must be reachable from the program"
    );

    for item_id in ast.program().items() {
        let item = ast.item(*item_id);
        assert_span(fixture, item.span);
        match &item.kind {
            ItemKind::Func(declaration) => {
                assert_span(fixture, declaration.name_span);
                if let Some(ret) = declaration.ret {
                    check_type(fixture, ast, ret, &mut types);
                }
                for param_id in &declaration.params {
                    check_param(fixture, ast, *param_id, &mut types, &mut params);
                }
                check_block(
                    fixture,
                    ast,
                    declaration.body,
                    &mut stmts,
                    &mut blocks,
                    &mut exprs,
                    &mut types,
                );
            }
            ItemKind::Error => {}
        }
    }

    assert_eq!(
        exprs.len(),
        ast.expr_arena().len(),
        "unreachable expression nodes"
    );
    assert_eq!(
        stmts.len(),
        ast.stmt_arena().len(),
        "unreachable statement nodes"
    );
    assert_eq!(
        blocks.len(),
        ast.block_arena().len(),
        "unreachable block nodes"
    );
    assert_eq!(
        types.len(),
        ast.type_arena().len(),
        "unreachable type nodes"
    );
    assert_eq!(
        params.len(),
        ast.param_arena().len(),
        "unreachable parameter nodes"
    );
}

fn assert_span(fixture: &Fixture, span: Span) {
    assert!(
        fixture.file.contains_span(span),
        "span {span:?} is outside the file"
    );
}

fn check_param(
    fixture: &Fixture,
    ast: &Ast,
    id: ParamId,
    types: &mut BTreeSet<TypeId>,
    params: &mut BTreeSet<ParamId>,
) {
    assert!(params.insert(id), "parameter {id:?} visited twice");
    let param: &Param = ast.param(id);
    assert_span(fixture, param.span);
    assert_span(fixture, param.name_span);
    check_type(fixture, ast, param.ty, types);
}

fn check_block(
    fixture: &Fixture,
    ast: &Ast,
    id: BlockId,
    stmts: &mut BTreeSet<StmtId>,
    blocks: &mut BTreeSet<BlockId>,
    exprs: &mut BTreeSet<ExprId>,
    types: &mut BTreeSet<TypeId>,
) {
    if !blocks.insert(id) {
        return;
    }
    let block = ast.block(id);
    assert_span(fixture, block.span);
    for statement in &block.statements {
        check_stmt(fixture, ast, *statement, stmts, exprs, types);
    }
    if let Some(tail) = block.tail {
        check_expr(fixture, ast, tail, exprs);
    }
}

fn check_stmt(
    fixture: &Fixture,
    ast: &Ast,
    id: StmtId,
    stmts: &mut BTreeSet<StmtId>,
    exprs: &mut BTreeSet<ExprId>,
    types: &mut BTreeSet<TypeId>,
) {
    assert!(stmts.insert(id), "statement {id:?} visited twice");
    let statement: &Stmt = ast.stmt(id);
    assert_span(fixture, statement.span);
    match &statement.kind {
        StmtKind::Let(binding) => {
            assert_span(fixture, binding.name_span);
            if let Some(ty) = binding.ty {
                check_type(fixture, ast, ty, types);
            }
            if let Some(init) = binding.init {
                check_expr(fixture, ast, init, exprs);
            }
        }
        StmtKind::Return(value) => {
            if let Some(value) = value {
                check_expr(fixture, ast, *value, exprs);
            }
        }
        StmtKind::Expr(expr) => check_expr(fixture, ast, *expr, exprs),
        StmtKind::Error => {}
    }
}

fn check_expr(fixture: &Fixture, ast: &Ast, id: ExprId, exprs: &mut BTreeSet<ExprId>) {
    if !exprs.insert(id) {
        return;
    }
    let expr = ast.expr(id);
    assert_span(fixture, expr.span);
    match &expr.kind {
        ExprKind::Unary { operand, .. } => check_expr(fixture, ast, *operand, exprs),
        ExprKind::Binary { lhs, rhs, .. } => {
            check_expr(fixture, ast, *lhs, exprs);
            check_expr(fixture, ast, *rhs, exprs);
        }
        ExprKind::Assign { target, value, .. } => {
            check_expr(fixture, ast, *target, exprs);
            check_expr(fixture, ast, *value, exprs);
        }
        ExprKind::Update { operand, .. } => check_expr(fixture, ast, *operand, exprs),
        ExprKind::Call { callee, args } => {
            check_expr(fixture, ast, *callee, exprs);
            for arg in args {
                check_expr(fixture, ast, *arg, exprs);
            }
        }
        ExprKind::Paren(inner) => check_expr(fixture, ast, *inner, exprs),
        ExprKind::Lit(_) | ExprKind::Path(_) | ExprKind::Error => {}
    }
}

fn check_type(fixture: &Fixture, ast: &Ast, id: TypeId, types: &mut BTreeSet<TypeId>) {
    if !types.insert(id) {
        return;
    }
    let ty: &Type = ast.ty(id);
    assert_span(fixture, ty.span);
    if let TypeKind::Pointer(inner) = ty.kind {
        check_type(fixture, ast, inner, types);
    }
}
