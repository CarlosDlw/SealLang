//! Fixtures and structural checks for parser tests.

use std::collections::BTreeSet;
use std::io;
use std::sync::{Arc, Mutex};

use seal_ast::{
    Ast, Block, BlockId, Expr, ExprId, ExprKind, FieldDeclId, FnDecl, ItemKind, ParamId, StmtId,
    StmtKind, TypeId, TypeKind,
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
    let mut reachability = Reachability {
        fixture,
        ast,
        exprs: BTreeSet::new(),
        stmts: BTreeSet::new(),
        blocks: BTreeSet::new(),
        fields: BTreeSet::new(),
        types: BTreeSet::new(),
        params: BTreeSet::new(),
    };
    reachability.run();
}

/// Walks the tree once, recording every node it sees.
struct Reachability<'a> {
    fixture: &'a Fixture,
    ast: &'a Ast,
    exprs: BTreeSet<ExprId>,
    stmts: BTreeSet<StmtId>,
    blocks: BTreeSet<BlockId>,
    fields: BTreeSet<FieldDeclId>,
    types: BTreeSet<TypeId>,
    params: BTreeSet<ParamId>,
}

impl Reachability<'_> {
    fn run(&mut self) {
        assert_eq!(
            self.ast.item_arena().len(),
            self.ast.program().len(),
            "every item must be reachable from the program"
        );

        for item_id in self.ast.program().items() {
            let item = self.ast.item(*item_id);
            self.assert_span(item.span);
            match &item.kind {
                ItemKind::Func(declaration) => {
                    self.assert_span(declaration.name_span);
                    if let Some(ret) = declaration.ret {
                        self.check_type(ret);
                    }
                    for param_id in &declaration.params {
                        self.check_param(*param_id);
                    }
                    self.check_block(declaration.body);
                }
                ItemKind::Struct(declaration) => {
                    self.assert_span(declaration.name_span);
                    for field in &declaration.fields {
                        self.check_field(*field);
                    }
                }
                ItemKind::Error => {}
            }
        }

        assert_eq!(
            self.exprs.len(),
            self.ast.expr_arena().len(),
            "unreachable expression nodes"
        );
        assert_eq!(
            self.stmts.len(),
            self.ast.stmt_arena().len(),
            "unreachable statement nodes"
        );
        assert_eq!(
            self.blocks.len(),
            self.ast.block_arena().len(),
            "unreachable block nodes"
        );
        assert_eq!(
            self.fields.len(),
            self.ast.field_arena().len(),
            "unreachable field nodes"
        );
        assert_eq!(
            self.types.len(),
            self.ast.type_arena().len(),
            "unreachable type nodes"
        );
        assert_eq!(
            self.params.len(),
            self.ast.param_arena().len(),
            "unreachable parameter nodes"
        );
    }

    fn assert_span(&self, span: Span) {
        assert!(
            self.fixture.file.contains_span(span),
            "span {span:?} is outside the file"
        );
    }

    fn check_param(&mut self, id: ParamId) {
        assert!(self.params.insert(id), "parameter {id:?} visited twice");
        let (span, name_span, ty) = {
            let param = self.ast.param(id);
            (param.span, param.name_span, param.ty)
        };
        self.assert_span(span);
        self.assert_span(name_span);
        self.check_type(ty);
    }

    fn check_field(&mut self, id: FieldDeclId) {
        assert!(self.fields.insert(id), "field {id:?} visited twice");
        let (span, name_span, ty) = {
            let field = self.ast.field(id);
            (field.span, field.name_span, field.ty)
        };
        self.assert_span(span);
        self.assert_span(name_span);
        self.check_type(ty);
    }

    fn check_block(&mut self, id: BlockId) {
        if !self.blocks.insert(id) {
            return;
        }
        let (span, statement_count, tail) = {
            let block = self.ast.block(id);
            (block.span, block.statements.len(), block.tail)
        };
        self.assert_span(span);
        for index in 0..statement_count {
            let statement = self.ast.block(id).statements[index];
            self.check_stmt(statement);
        }
        if let Some(tail) = tail {
            self.check_expr(tail);
        }
    }

    fn check_stmt(&mut self, id: StmtId) {
        assert!(self.stmts.insert(id), "statement {id:?} visited twice");
        let (span, kind) = {
            let statement = self.ast.stmt(id);
            (statement.span, statement.kind.clone())
        };
        self.assert_span(span);
        match kind {
            StmtKind::Let(binding) => {
                self.assert_span(binding.name_span);
                if let Some(ty) = binding.ty {
                    self.check_type(ty);
                }
                if let Some(init) = binding.init {
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
            StmtKind::Expr(expr) => self.check_expr(expr),
            StmtKind::Break | StmtKind::Continue | StmtKind::Error => {}
        }
    }

    fn check_expr(&mut self, id: ExprId) {
        if !self.exprs.insert(id) {
            return;
        }
        let (span, kind) = {
            let expr = self.ast.expr(id);
            (expr.span, expr.kind.clone())
        };
        self.assert_span(span);
        match kind {
            ExprKind::Unary { operand, .. }
            | ExprKind::Borrow { operand, .. }
            | ExprKind::Deref { operand } => self.check_expr(operand),
            ExprKind::Field {
                base, name_span, ..
            } => {
                self.assert_span(name_span);
                self.check_expr(base);
            }
            ExprKind::Array { elements } => {
                for element in elements {
                    self.check_expr(element);
                }
            }
            ExprKind::ArrayRepeat {
                value, length_span, ..
            } => {
                self.assert_span(length_span);
                self.check_expr(value);
            }
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
            ExprKind::Struct {
                name_span, fields, ..
            } => {
                self.assert_span(name_span);
                for field in fields {
                    self.assert_span(field.name_span);
                    self.assert_span(field.span);
                    self.check_expr(field.value);
                }
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                self.check_expr(lhs);
                self.check_expr(rhs);
            }
            ExprKind::Assign { target, value, .. } => {
                self.check_expr(target);
                self.check_expr(value);
            }
            ExprKind::Update { operand, .. } => self.check_expr(operand),
            ExprKind::Call { callee, args } => {
                self.check_expr(callee);
                for arg in args {
                    self.check_expr(arg);
                }
            }
            ExprKind::Paren(inner) => self.check_expr(inner),
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
            ExprKind::Lit(_) | ExprKind::Path(_) | ExprKind::Error => {}
        }
    }

    fn check_type(&mut self, id: TypeId) {
        if !self.types.insert(id) {
            return;
        }
        let (span, kind) = {
            let ty = self.ast.ty(id);
            (ty.span, ty.kind.clone())
        };
        self.assert_span(span);
        match kind {
            TypeKind::Pointer { pointee, .. } => self.check_type(pointee),
            TypeKind::Array {
                element,
                length_span,
                ..
            } => {
                self.assert_span(length_span);
                self.check_type(element);
            }
            TypeKind::Slice { element } => self.check_type(element),
            _ => {}
        }
    }
}
