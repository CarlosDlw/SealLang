//! Fixtures and structural checks for HIR tests.

use std::collections::BTreeSet;
use std::io;
use std::sync::{Arc, Mutex};

use crate::{
    Body, BodyId, ExprKind, Hir, HirBlockId, HirExprId, HirPatternId, HirStmtId, PatternKind, Res,
    StmtKind,
};
use seal_ast::Ast;
use seal_def::{DefId, DefKind};
use seal_diagnostics::{DiagCtxt, Diagnostic, Emitter};
use seal_span::{FileName, SourceFile, SourceMap, Span};
use seal_symbol::{Interner, Symbol};
use seal_ty::TyId;

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

    fn parse(&self) -> Ast {
        let lexed = seal_lexer::lex(&self.file, &self.dcx, &self.interner);
        seal_parser::parse(&lexed, &self.dcx, &self.interner)
    }

    pub(crate) fn lower(&self) -> Hir {
        let ast = self.parse();
        crate::lower(&ast, &self.dcx, &self.interner)
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

pub(crate) fn lower_clean(source: &str) -> (Fixture, Hir) {
    let fixture = Fixture::new(source);
    let hir = fixture.lower();
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
            assert_eq!(
                owner.kind,
                DefKind::Function,
                "a body must be owned by a function"
            );
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

        if let Some(place) = self.hir.place(id) {
            let data = self.hir.definitions().get(place.local_def());
            assert!(data.kind.is_local(), "a place must name a local");
            assert_eq!(data.parent, self.owner);
        }

        match kind {
            ExprKind::Unary { operand, .. }
            | ExprKind::Borrow { operand, .. }
            | ExprKind::Deref { operand } => self.check_expr(operand),
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
            ExprKind::Len { operand } => self.check_expr(operand),
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
            ExprKind::Block(block) => self.check_block(block),
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
            ExprKind::Path { args, .. } => self.check_optional_substs(args),
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
