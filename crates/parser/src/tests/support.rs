//! Fixtures and structural checks for parser tests.

use std::collections::BTreeSet;
use std::io;
use std::sync::{Arc, Mutex};

use seal_ast::{
    Ast, Block, BlockId, BoundId, BoundKind, Expr, ExprId, ExprKind, FieldDeclId, FnDecl,
    GenericArgs, GenericParamId, GenericParams, ItemId, ItemKind, ParamId, PatternId, PatternKind,
    StmtId, StmtKind, TupleFieldDeclId, TypeId, TypeKind, VariantDeclId, VariantFieldList,
    VariantPatternFields, WhereClause,
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
        generic_params: BTreeSet::new(),
        bounds: BTreeSet::new(),
        fields: BTreeSet::new(),
        tuple_fields: BTreeSet::new(),
        variants: BTreeSet::new(),
        types: BTreeSet::new(),
        params: BTreeSet::new(),
        items: BTreeSet::new(),
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
    generic_params: BTreeSet<GenericParamId>,
    bounds: BTreeSet<BoundId>,
    fields: BTreeSet<FieldDeclId>,
    tuple_fields: BTreeSet<TupleFieldDeclId>,
    variants: BTreeSet<VariantDeclId>,
    types: BTreeSet<TypeId>,
    params: BTreeSet<ParamId>,
    items: BTreeSet<ItemId>,
}

impl Reachability<'_> {
    fn run(&mut self) {
        for item_id in self.ast.program().items() {
            self.check_item(*item_id);
        }

        assert_eq!(
            self.items.len(),
            self.ast.item_arena().len(),
            "unreachable item nodes"
        );

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
            self.tuple_fields.len(),
            self.ast.tuple_field_arena().len(),
            "unreachable tuple field nodes"
        );
        assert_eq!(
            self.variants.len(),
            self.ast.variant_arena().len(),
            "unreachable variant nodes"
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
        assert_eq!(
            self.generic_params.len(),
            self.ast.generic_param_arena().len(),
            "unreachable generic parameter nodes"
        );
        assert_eq!(
            self.bounds.len(),
            self.ast.bound_arena().len(),
            "unreachable bound nodes"
        );
    }

    fn check_item(&mut self, id: ItemId) {
        assert!(self.items.insert(id), "item {id:?} visited twice");
        let item = self.ast.item(id);
        let span = item.span;
        let kind = item.kind.clone();
        self.assert_span(span);
        match kind {
            ItemKind::Func(declaration) => self.check_function(&declaration),
            ItemKind::Struct(declaration) => {
                self.assert_span(declaration.name_span);
                self.check_generics(&declaration.generics);
                self.check_where(&declaration.where_clause);
                for field in &declaration.fields {
                    self.check_field(*field);
                }
            }
            ItemKind::Enum(declaration) => {
                self.assert_span(declaration.name_span);
                self.check_generics(&declaration.generics);
                self.check_where(&declaration.where_clause);
                for variant in &declaration.variants {
                    self.check_variant(*variant);
                }
            }
            ItemKind::Impl(declaration) => {
                self.check_generics(&declaration.generics);
                self.check_type(declaration.self_ty);
                if let Some(trait_ref) = &declaration.trait_ref {
                    self.assert_span(trait_ref.name_span);
                    self.assert_span(trait_ref.span);
                    if let Some(args) = &trait_ref.args {
                        self.check_generic_args(args);
                    }
                }
                self.check_where(&declaration.where_clause);
                for function in &declaration.functions {
                    self.check_item(*function);
                }
            }
            ItemKind::Trait(declaration) => {
                self.assert_span(declaration.name_span);
                self.check_generics(&declaration.generics);
                self.check_where(&declaration.where_clause);
                for method in &declaration.methods {
                    self.check_item(*method);
                }
            }
            ItemKind::TraitMethod(declaration) => {
                self.assert_span(declaration.name_span);
                self.check_generics(&declaration.generics);
                if let Some(ret) = declaration.ret {
                    self.check_type(ret);
                }
                self.check_where(&declaration.where_clause);
                for param in &declaration.params {
                    self.check_param(*param);
                }
                if let Some(body) = declaration.body {
                    self.check_block(body);
                }
            }
            ItemKind::Const(declaration) => {
                self.assert_span(declaration.name_span);
                self.check_type(declaration.ty);
                self.check_expr(declaration.init);
            }
            ItemKind::Static(declaration) => {
                self.assert_span(declaration.name_span);
                self.check_type(declaration.ty);
                self.check_expr(declaration.init);
            }
            ItemKind::Error => {}
        }
    }

    fn check_variant(&mut self, id: VariantDeclId) {
        assert!(self.variants.insert(id), "variant {id:?} visited twice");
        let (span, name_span, fields) = {
            let variant = self.ast.variant(id);
            (variant.span, variant.name_span, variant.fields.clone())
        };
        self.assert_span(span);
        self.assert_span(name_span);
        match fields {
            VariantFieldList::Unit => {}
            VariantFieldList::Named(field_ids) => {
                for field in field_ids {
                    self.check_field(field);
                }
            }
            VariantFieldList::Tuple(field_ids) => {
                for field in field_ids {
                    self.check_tuple_field(field);
                }
            }
        }
    }

    fn check_tuple_field(&mut self, id: TupleFieldDeclId) {
        assert!(
            self.tuple_fields.insert(id),
            "tuple field {id:?} visited twice"
        );
        let (span, ty) = {
            let field = self.ast.tuple_field(id);
            (field.span, field.ty)
        };
        self.assert_span(span);
        self.check_type(ty);
    }

    fn check_function(&mut self, declaration: &FnDecl) {
        self.assert_span(declaration.name_span);
        self.check_generics(&declaration.generics);
        if let Some(ret) = declaration.ret {
            self.check_type(ret);
        }
        self.check_where(&declaration.where_clause);
        for param_id in &declaration.params {
            self.check_param(*param_id);
        }
        self.check_block(declaration.body);
    }

    fn check_generics(&mut self, generics: &GenericParams) {
        self.assert_span(generics.span);
        for id in &generics.params {
            assert!(
                self.generic_params.insert(*id),
                "generic parameter {id:?} visited twice"
            );
            let (name_span, span, bounds) = {
                let param = self.ast.generic_param(*id);
                (param.name_span, param.span, param.bounds.clone())
            };
            self.assert_span(name_span);
            self.assert_span(span);
            for bound in bounds {
                self.check_bound(bound);
            }
        }
    }

    fn check_where(&mut self, clause: &WhereClause) {
        for predicate in &clause.predicates {
            self.assert_span(predicate.span);
            self.check_type(predicate.ty);
            for bound in &predicate.bounds {
                self.check_bound(*bound);
            }
        }
    }

    fn check_bound(&mut self, id: BoundId) {
        assert!(self.bounds.insert(id), "bound {id:?} visited twice");
        let (span, kind) = {
            let bound = self.ast.bound(id);
            (bound.span, bound.kind.clone())
        };
        self.assert_span(span);
        if let BoundKind::Trait(trait_ref) = kind {
            self.assert_span(trait_ref.name_span);
            self.assert_span(trait_ref.span);
            if let Some(args) = &trait_ref.args {
                self.check_generic_args(args);
            }
        }
    }

    fn check_generic_args(&mut self, args: &GenericArgs) {
        self.assert_span(args.span);
        for arg in &args.args {
            self.check_type(*arg);
        }
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
            StmtKind::Const(declaration) => {
                self.assert_span(declaration.name_span);
                self.check_type(declaration.ty);
                self.check_expr(declaration.init);
            }
            StmtKind::Static(declaration) => {
                self.assert_span(declaration.name_span);
                self.check_type(declaration.ty);
                self.check_expr(declaration.init);
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
                    seal_ast::ForSource::Range { start, end, .. } => {
                        self.check_expr(start);
                        self.check_expr(end);
                    }
                    seal_ast::ForSource::Expr(expr) => self.check_expr(expr),
                }
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
            ExprKind::TupleField {
                base, index_span, ..
            } => {
                self.assert_span(index_span);
                self.check_expr(base);
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
                name_span,
                type_args,
                fields,
                ..
            } => {
                self.assert_span(name_span);
                if let Some(args) = type_args {
                    self.check_generic_args(&args);
                }
                for field in fields {
                    self.assert_span(field.name_span);
                    self.assert_span(field.span);
                    self.check_expr(field.value);
                }
            }
            ExprKind::Variant {
                type_span,
                name_span,
                type_args,
                fields,
                ..
            } => {
                self.assert_span(type_span);
                self.assert_span(name_span);
                if let Some(args) = type_args {
                    self.check_generic_args(&args);
                }
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
            ExprKind::Cast {
                operand,
                ty,
                as_span,
            } => {
                self.assert_span(as_span);
                self.check_expr(operand);
                self.assert_span(self.ast.ty(ty).span);
                self.check_type(ty);
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
            ExprKind::MethodCall {
                receiver,
                name_span,
                generic_args,
                args,
                ..
            } => {
                self.assert_span(name_span);
                if let Some(args) = generic_args {
                    self.check_generic_args(&args);
                }
                self.check_expr(receiver);
                for arg in args {
                    self.check_expr(arg);
                }
            }
            ExprKind::Paren(inner) => self.check_expr(inner),
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
            ExprKind::Block(block) | ExprKind::Unsafe(block) => self.check_block(block),
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
            ExprKind::PathQualified {
                type_span,
                name_span,
                type_args,
                ..
            } => {
                self.assert_span(type_span);
                self.assert_span(name_span);
                if let Some(args) = type_args {
                    self.check_generic_args(&args);
                }
            }
            ExprKind::Path {
                name_span, args, ..
            } => {
                self.assert_span(name_span);
                if let Some(args) = args {
                    self.check_generic_args(&args);
                }
            }
            ExprKind::Lit(_) | ExprKind::Error => {}
        }
    }

    fn check_pattern(&mut self, id: PatternId) {
        let (span, kind) = {
            let pattern = self.ast.pattern(id);
            (pattern.span, pattern.kind.clone())
        };
        self.assert_span(span);
        match kind {
            PatternKind::Variant {
                type_span,
                name_span,
                fields,
                ..
            } => {
                self.assert_span(type_span);
                self.assert_span(name_span);
                match fields {
                    VariantPatternFields::Unit => {}
                    VariantPatternFields::Positional(patterns) => {
                        for pattern in patterns {
                            self.check_pattern(pattern);
                        }
                    }
                    VariantPatternFields::Named { fields, .. } => {
                        for field in fields {
                            self.assert_span(field.name_span);
                            self.assert_span(field.span);
                            self.check_pattern(field.pattern);
                        }
                    }
                }
            }
            PatternKind::Struct {
                name_span, fields, ..
            } => {
                self.assert_span(name_span);
                for field in fields {
                    self.assert_span(field.name_span);
                    self.assert_span(field.span);
                    self.check_pattern(field.pattern);
                }
            }
            PatternKind::Binding { pattern, .. } => {
                if let Some(pattern) = pattern {
                    self.check_pattern(pattern);
                }
            }
            PatternKind::Or(alternatives) => {
                for alternative in alternatives {
                    self.check_pattern(alternative);
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
            PatternKind::Wildcard
            | PatternKind::Literal(_)
            | PatternKind::Range { .. }
            | PatternKind::Error => {}
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
            TypeKind::Pointer { pointee, .. } | TypeKind::RawPointer { pointee, .. } => {
                self.check_type(pointee)
            }
            TypeKind::Array {
                element,
                length_span,
                ..
            } => {
                self.assert_span(length_span);
                self.check_type(element);
            }
            TypeKind::Slice { element } => self.check_type(element),
            TypeKind::Tuple(elements) => {
                for element in elements {
                    self.check_type(element);
                }
            }
            TypeKind::Path(path) => {
                self.assert_span(path.name_span);
                if let Some(args) = &path.args {
                    self.check_generic_args(args);
                }
            }
            TypeKind::Dyn(path) => {
                self.assert_span(path.name_span);
                self.assert_span(path.span);
                if let Some(args) = &path.args {
                    self.check_generic_args(args);
                }
            }
            _ => {}
        }
    }
}
