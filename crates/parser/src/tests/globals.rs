//! Constants, statics, `const func`, and `unsafe` blocks.

use seal_ast::{ExprKind, ItemKind, StmtKind, TypeKind};

use crate::tests::support::{Fixture, assert_invariants, body, parse_clean};

fn item(ast: &seal_ast::Ast, index: usize) -> &seal_ast::Item {
    ast.item(ast.program().items()[index])
}

#[test]
fn file_scope_const_and_static_parse() {
    let source = "const LIMIT: usize = 1 << 20;\n\
                  static GREETING: *str = \"hello\";\n\
                  static TABLE: [i32; 4] = [1, 2, 3, 4];";
    let (fixture, ast) = parse_clean(source);

    let ItemKind::Const(limit) = &item(&ast, 0).kind else {
        panic!("first item is not a constant: {:?}", item(&ast, 0).kind);
    };
    assert_eq!(fixture.symbol(limit.name), "LIMIT");
    assert_eq!(fixture.text(limit.name_span), "LIMIT");
    assert!(matches!(
        ast.ty(limit.ty).kind,
        TypeKind::Primitive(seal_ast::PrimitiveType::Usize)
    ));
    assert!(matches!(ast.expr(limit.init).kind, ExprKind::Binary { .. }));
    assert_eq!(
        fixture.text(item(&ast, 0).span),
        "const LIMIT: usize = 1 << 20;"
    );

    let ItemKind::Static(greeting) = &item(&ast, 1).kind else {
        panic!("second item is not a static: {:?}", item(&ast, 1).kind);
    };
    assert_eq!(fixture.symbol(greeting.name), "GREETING");
    assert!(!greeting.mutable);
    assert!(matches!(ast.expr(greeting.init).kind, ExprKind::Lit(_)));
    assert_eq!(
        fixture.text(item(&ast, 1).span),
        "static GREETING: *str = \"hello\";"
    );

    let ItemKind::Static(table) = &item(&ast, 2).kind else {
        panic!("third item is not a static: {:?}", item(&ast, 2).kind);
    };
    assert!(matches!(ast.expr(table.init).kind, ExprKind::Array { .. }));
    assert_eq!(
        fixture.text(item(&ast, 2).span),
        "static TABLE: [i32; 4] = [1, 2, 3, 4];"
    );
}

#[test]
fn static_mut_is_marked_mutable() {
    let (fixture, ast) = parse_clean("static mut COUNTER: i32 = 0;");
    let ItemKind::Static(declaration) = &item(&ast, 0).kind else {
        panic!("item is not a static: {:?}", item(&ast, 0).kind);
    };
    assert!(declaration.mutable);
    assert_eq!(fixture.symbol(declaration.name), "COUNTER");
    assert_eq!(
        fixture.text(item(&ast, 0).span),
        "static mut COUNTER: i32 = 0;"
    );
}

#[test]
fn const_function_carries_the_flag_and_the_prefix_span() {
    let source = "const func twice(value: i32) -> i32 { value * 2 }\nfunc plain() {}";
    let (fixture, ast) = parse_clean(source);

    let ItemKind::Func(twice) = &item(&ast, 0).kind else {
        panic!("first item is not a function: {:?}", item(&ast, 0).kind);
    };
    assert!(twice.is_const);
    assert_eq!(fixture.symbol(twice.name), "twice");
    assert_eq!(twice.params.len(), 1);
    assert!(matches!(
        ast.expr(body(&ast, twice).tail.expect("tail")).kind,
        ExprKind::Binary { .. }
    ));
    assert_eq!(
        fixture.text(item(&ast, 0).span),
        "const func twice(value: i32) -> i32 { value * 2 }"
    );

    let ItemKind::Func(plain) = &item(&ast, 1).kind else {
        panic!("second item is not a function: {:?}", item(&ast, 1).kind);
    };
    assert!(!plain.is_const);
}

#[test]
fn block_scope_declarations_are_statements() {
    let source = "func example() -> i32 {\n\
                  const SCALE: i32 = 3;\n\
                  static mut CALLS: i32 = 0;\n\
                  SCALE\n\
                  }";
    let (fixture, ast) = parse_clean(source);
    let ItemKind::Func(function) = &item(&ast, 0).kind else {
        panic!("item is not a function: {:?}", item(&ast, 0).kind);
    };
    let block = body(&ast, function);
    assert_eq!(block.statements.len(), 2);

    let first = ast.stmt(block.statements[0]);
    let StmtKind::Const(scale) = &first.kind else {
        panic!("first statement is not a constant: {:?}", first.kind);
    };
    assert_eq!(fixture.symbol(scale.name), "SCALE");
    assert_eq!(fixture.text(first.span), "const SCALE: i32 = 3;");
    assert!(matches!(ast.expr(scale.init).kind, ExprKind::Lit(_)));

    let second = ast.stmt(block.statements[1]);
    let StmtKind::Static(calls) = &second.kind else {
        panic!("second statement is not a static: {:?}", second.kind);
    };
    assert!(calls.mutable);
    assert_eq!(fixture.symbol(calls.name), "CALLS");
    assert_eq!(fixture.text(second.span), "static mut CALLS: i32 = 0;");
}

#[test]
fn unsafe_block_is_an_expression_with_the_block_value() {
    let source = "func main() -> i32 { let value = unsafe { 1 }; value }";
    let (fixture, ast) = parse_clean(source);
    let ItemKind::Func(function) = &item(&ast, 0).kind else {
        panic!("item is not a function: {:?}", item(&ast, 0).kind);
    };
    let statement = ast.stmt(body(&ast, function).statements[0]);
    let StmtKind::Let(binding) = &statement.kind else {
        panic!("statement is not a binding: {:?}", statement.kind);
    };
    let init = ast.expr(binding.init.expect("initializer"));
    let ExprKind::Unsafe(block) = init.kind else {
        panic!("initializer is not an unsafe block: {:?}", init.kind);
    };
    assert_eq!(fixture.text(init.span), "unsafe { 1 }");
    let block = ast.block(block);
    assert!(matches!(
        ast.expr(block.tail.expect("block tail")).kind,
        ExprKind::Lit(_)
    ));
}

#[test]
fn unsafe_block_statement_needs_no_semicolon() {
    let source = "func main() -> i32 { unsafe { } 1 }";
    let (_, ast) = parse_clean(source);
    let ItemKind::Func(function) = &item(&ast, 0).kind else {
        panic!("item is not a function: {:?}", item(&ast, 0).kind);
    };
    let block = body(&ast, function);
    assert_eq!(block.statements.len(), 1);
    let statement = ast.stmt(block.statements[0]);
    let StmtKind::Expr(expression) = statement.kind else {
        panic!("statement is not an expression: {:?}", statement.kind);
    };
    assert!(matches!(ast.expr(expression).kind, ExprKind::Unsafe(_)));
    assert!(block.tail.is_some());
}

#[test]
fn missing_type_is_reported_once_and_the_node_is_total() {
    let fixture = Fixture::new("const LIMIT = 10;");
    let ast = fixture.parse();
    fixture.expect("E1012", "needs an explicit type");
    assert_eq!(fixture.codes(), ["E1012"]);
    let ItemKind::Const(declaration) = &item(&ast, 0).kind else {
        panic!("item is not a constant: {:?}", item(&ast, 0).kind);
    };
    assert!(matches!(ast.ty(declaration.ty).kind, TypeKind::Error));
    assert!(matches!(ast.expr(declaration.init).kind, ExprKind::Lit(_)));
}

#[test]
fn missing_initializer_is_reported_once_and_the_node_is_total() {
    let fixture = Fixture::new("static COUNTER: i32;");
    let ast = fixture.parse();
    fixture.expect("E1012", "needs an initializer");
    assert_eq!(fixture.codes(), ["E1012"]);
    let ItemKind::Static(declaration) = &item(&ast, 0).kind else {
        panic!("item is not a static: {:?}", item(&ast, 0).kind);
    };
    assert!(matches!(ast.expr(declaration.init).kind, ExprKind::Error));
}

#[test]
fn missing_type_and_initializer_is_one_diagnostic() {
    let fixture = Fixture::new("const X;");
    let ast = fixture.parse();
    fixture.expect("E1012", "needs an explicit type and an initializer");
    assert_eq!(fixture.codes(), ["E1012"]);
    assert_invariants(&fixture, &ast);
}

#[test]
fn const_mut_is_rejected_but_the_constant_remains() {
    let fixture = Fixture::new("const mut LIMIT: usize = 1;");
    let ast = fixture.parse();
    fixture.expect("E1013", "`const` cannot be declared mutable");
    assert_eq!(fixture.codes(), ["E1013"]);
    let ItemKind::Const(declaration) = &item(&ast, 0).kind else {
        panic!("item is not a constant: {:?}", item(&ast, 0).kind);
    };
    assert_eq!(fixture.symbol(declaration.name), "LIMIT");
}

#[test]
fn a_second_mut_is_rejected() {
    let fixture = Fixture::new("static mut mut COUNTER: i32 = 0;");
    let ast = fixture.parse();
    fixture.expect("E1013", "at most one");
    assert_eq!(fixture.codes(), ["E1013"]);
    let ItemKind::Static(declaration) = &item(&ast, 0).kind else {
        panic!("item is not a static: {:?}", item(&ast, 0).kind);
    };
    assert!(declaration.mutable);
}

#[test]
fn static_function_is_rejected_as_a_function() {
    let fixture = Fixture::new("static func value() -> i32 { 1 }");
    let ast = fixture.parse();
    fixture.expect("E1013", "`static` cannot declare a function");
    assert_eq!(fixture.codes(), ["E1013"]);
    let ItemKind::Func(declaration) = &item(&ast, 0).kind else {
        panic!("item is not a function: {:?}", item(&ast, 0).kind);
    };
    assert_eq!(fixture.symbol(declaration.name), "value");
    assert!(!declaration.is_const);
}

#[test]
fn local_const_function_is_rejected_and_skipped() {
    let source = "func example() { const func inner() -> i32 { 1 } 2 }";
    let fixture = Fixture::new(source);
    let ast = fixture.parse();
    fixture.expect("E1014", "`const func` cannot be declared inside a block");
    assert_eq!(fixture.codes(), ["E1014"]);
    let ItemKind::Func(function) = &item(&ast, 0).kind else {
        panic!("item is not a function: {:?}", item(&ast, 0).kind);
    };
    let block = body(&ast, function);
    assert_eq!(block.statements.len(), 1);
    assert!(matches!(
        ast.stmt(block.statements[0]).kind,
        StmtKind::Error
    ));
    assert!(
        block.tail.is_some(),
        "the value after the declaration is kept"
    );
    assert_invariants(&fixture, &ast);
}

#[test]
fn a_missing_semicolon_is_reported_in_context() {
    let fixture = Fixture::new("const LIMIT: usize = 1");
    fixture.parse();
    fixture.expect("E1002", "expected `;` after the `const` declaration");
}

#[test]
fn a_missing_name_is_reported_and_the_declaration_survives() {
    let fixture = Fixture::new("const : i32 = 1;");
    let ast = fixture.parse();
    fixture.expect("E1005", "expected a declaration name");
    let ItemKind::Const(declaration) = &item(&ast, 0).kind else {
        panic!("item is not a constant: {:?}", item(&ast, 0).kind);
    };
    assert_eq!(fixture.symbol(declaration.name), "");
}

#[test]
fn a_reserved_keyword_cannot_name_a_value_declaration() {
    for (source, keyword) in [
        ("const unsafe: i32 = 1;", "unsafe"),
        ("static const: i32 = 1;", "const"),
    ] {
        let fixture = Fixture::new(source);
        let _ = fixture.parse();
        fixture.expect("E1005", "a declaration name");
        assert!(
            fixture.diagnostics().iter().any(|diagnostic| diagnostic
                .helps()
                .iter()
                .any(|help| help.contains(&format!("`{keyword}` is a reserved keyword")))),
            "the diagnostic explains that `{keyword}` is reserved: {:?}",
            fixture.diagnostics()
        );
    }
}

#[test]
fn unsafe_without_a_block_is_reported() {
    let fixture = Fixture::new("func main() { unsafe 1 }");
    let ast = fixture.parse();
    fixture.expect("E1002", "expected `{`");
    let ItemKind::Func(function) = &item(&ast, 0).kind else {
        panic!("item is not a function: {:?}", item(&ast, 0).kind);
    };
    let block = body(&ast, function);
    let StmtKind::Expr(expression) = ast.stmt(block.statements[0]).kind else {
        panic!("statement is not an expression");
    };
    let ExprKind::Unsafe(empty) = ast.expr(expression).kind else {
        panic!("expression is not an unsafe block");
    };
    assert!(ast.block(empty).statements.is_empty());
    assert_invariants(&fixture, &ast);
}

#[test]
fn recovery_continues_at_the_next_const() {
    let source = "const BAD = ;\nconst GOOD: i32 = 2;\nfunc main() -> i32 { GOOD }";
    let fixture = Fixture::new(source);
    let ast = fixture.parse();
    fixture.expect("E1012", "needs an explicit type");
    assert_eq!(ast.program().len(), 3, "every declaration is kept");
    let ItemKind::Const(good) = &item(&ast, 1).kind else {
        panic!("second item is not a constant: {:?}", item(&ast, 1).kind);
    };
    assert_eq!(fixture.symbol(good.name), "GOOD");
    assert_invariants(&fixture, &ast);
}

#[test]
fn globals_keep_tree_invariants_on_hostile_input() {
    let sources = [
        "const",
        "static mut",
        "const func",
        "static func",
        "const A: = 1;",
        "static B: i32 = ;",
        "func main() { const func f() { } }",
        "func main() { unsafe { const X: i32 = 1; } }",
        "const func f<T>(value: T) -> T { value }",
    ];
    for source in sources {
        let fixture = Fixture::new(source);
        let ast = fixture.parse();
        assert_invariants(&fixture, &ast);
    }
}
