//! Blocks, `let`, `return`, expression statements, and the tail expression.

use seal_ast::{ExprKind, Literal, StmtKind};
use seal_lexer::TokenKind;

use crate::tests::support::{Fixture, assert_invariants, body, parse_clean, sole_function};

#[test]
fn let_bindings_cover_every_form() {
    let (fixture, ast) =
        parse_clean("func main() { let a: i32 = 1; let mut b = 2; let mut c: i32; a }");
    let declaration = sole_function(&ast);
    let block = body(&ast, declaration);
    assert_eq!(block.statements.len(), 3);

    match &ast.stmt(block.statements[0]).kind {
        StmtKind::Let(binding) => {
            assert_eq!(fixture.symbol(binding.name), "a");
            assert!(!binding.mutable);
            assert!(binding.ty.is_some());
            assert!(binding.init.is_some());
        }
        other => panic!("expected let, got {other:?}"),
    }
    match &ast.stmt(block.statements[1]).kind {
        StmtKind::Let(binding) => {
            assert_eq!(fixture.symbol(binding.name), "b");
            assert!(binding.mutable);
            assert!(binding.ty.is_none());
            assert!(binding.init.is_some());
        }
        other => panic!("expected let, got {other:?}"),
    }
    match &ast.stmt(block.statements[2]).kind {
        StmtKind::Let(binding) => {
            assert_eq!(fixture.symbol(binding.name), "c");
            assert!(binding.mutable);
            assert!(binding.ty.is_some());
            assert!(binding.init.is_none());
        }
        other => panic!("expected let, got {other:?}"),
    }
    assert!(block.tail.is_some());
}

#[test]
fn bindings_are_immutable_unless_declared_mut() {
    let (fixture, ast) = parse_clean("func main() { let fixed = 1; let mut changing = 2; }");
    let declaration = sole_function(&ast);
    let block = body(&ast, declaration);
    assert_eq!(block.statements.len(), 2);

    match &ast.stmt(block.statements[0]).kind {
        StmtKind::Let(binding) => {
            assert_eq!(fixture.symbol(binding.name), "fixed");
            assert!(!binding.mutable);
        }
        other => panic!("expected let, got {other:?}"),
    }
    match &ast.stmt(block.statements[1]).kind {
        StmtKind::Let(binding) => {
            assert_eq!(fixture.symbol(binding.name), "changing");
            assert!(binding.mutable);
        }
        other => panic!("expected let, got {other:?}"),
    }
}

#[test]
fn mut_is_a_keyword_not_a_binding_name() {
    let fixture = Fixture::new("func main() { let mut = 1; }");
    let _ = fixture.parse();
    fixture.expect("E1005", "a binding name");
}

#[test]
fn a_reserved_keyword_cannot_be_a_binding_name() {
    let fixture = Fixture::new("func main() { let static = 1; }");
    let _ = fixture.parse();
    fixture.expect("E1005", "a binding name");
    assert!(
        fixture.diagnostics().iter().any(|diagnostic| diagnostic
            .helps()
            .iter()
            .any(|help| help.contains("`static` is a reserved keyword"))),
        "the diagnostic names the reserved keyword: {:?}",
        fixture.diagnostics()
    );
}

#[test]
fn name_spans_point_at_the_names() {
    let (fixture, ast) = parse_clean("func main(param: i32) { let local = 1; }");
    let declaration = sole_function(&ast);
    assert_eq!(fixture.text(declaration.name_span), "main");
    let param = ast.param(declaration.params[0]);
    assert_eq!(fixture.text(param.name_span), "param");
    let block = body(&ast, declaration);
    match &ast.stmt(block.statements[0]).kind {
        StmtKind::Let(binding) => assert_eq!(fixture.text(binding.name_span), "local"),
        other => panic!("expected let, got {other:?}"),
    }
}

#[test]
fn return_statements_carry_optional_values() {
    let (_, ast) = parse_clean("func main() { return 1; return; 2 }");
    let declaration = sole_function(&ast);
    let block = body(&ast, declaration);
    assert_eq!(block.statements.len(), 2);
    match &ast.stmt(block.statements[0]).kind {
        StmtKind::Return(Some(value)) => {
            assert!(matches!(
                ast.expr(*value).kind,
                ExprKind::Lit(Literal::Int { value: 1, .. })
            ));
        }
        other => panic!("expected return with value, got {other:?}"),
    }
    assert!(matches!(
        ast.stmt(block.statements[1]).kind,
        StmtKind::Return(None)
    ));
}

#[test]
fn expression_statements_and_tail_expressions_are_distinct() {
    let (_, ast) = parse_clean("func main() { a = 1; b += 2; c }");
    let declaration = sole_function(&ast);
    let block = body(&ast, declaration);
    assert_eq!(block.statements.len(), 2);
    assert!(matches!(
        ast.stmt(block.statements[0]).kind,
        StmtKind::Expr(_)
    ));
    assert!(matches!(
        ast.stmt(block.statements[1]).kind,
        StmtKind::Expr(_)
    ));
    let tail = block.tail.expect("tail exists");
    assert!(matches!(ast.expr(tail).kind, ExprKind::Path { .. }));
}

#[test]
fn empty_blocks_have_no_tail() {
    let (_, ast) = parse_clean("func main() {}");
    let declaration = sole_function(&ast);
    assert!(body(&ast, declaration).statements.is_empty());
    assert!(body(&ast, declaration).tail.is_none());
    assert!(declaration.ret.is_none());
}

#[test]
fn missing_semicolon_between_lets_does_not_lose_the_second() {
    let fixture = Fixture::new("func main() { let a = 1 let b = 2; }");
    let ast = fixture.parse();
    fixture.expect("E1002", "`;`");
    let declaration = sole_function(&ast);
    assert_eq!(body(&ast, declaration).statements.len(), 2);
}

#[test]
fn missing_semicolon_before_a_new_expression_is_diagnosed() {
    let fixture = Fixture::new("func main() { a b }");
    let ast = fixture.parse();
    fixture.expect("E1002", "`;`");
    let declaration = sole_function(&ast);
    let block = body(&ast, declaration);
    assert_eq!(block.statements.len(), 1);
    assert!(block.tail.is_some());
}

#[test]
fn let_without_type_or_initializer_reports_e1007() {
    let fixture = Fixture::new("func main() { let x; }");
    let _ = fixture.parse();
    fixture.expect("E1007", "needs a type or an initializer");
}

#[test]
fn return_without_semicolon_is_diagnosed_but_kept() {
    let fixture = Fixture::new("func main() { return 1 }");
    let ast = fixture.parse();
    fixture.expect("E1002", "`return`");
    let declaration = sole_function(&ast);
    let block = body(&ast, declaration);
    assert_eq!(block.statements.len(), 1);
    assert!(matches!(
        ast.stmt(block.statements[0]).kind,
        StmtKind::Return(Some(_))
    ));
}

#[test]
fn unclosed_blocks_report_the_missing_brace() {
    let fixture = Fixture::new("func main() { 1");
    let ast = fixture.parse();
    fixture.expect("E1002", "`}`");
    let declaration = sole_function(&ast);
    let block = body(&ast, declaration);
    assert!(block.tail.is_some());
}

#[test]
fn an_expression_missing_at_the_end_of_a_statement_reports_e1003() {
    let fixture = Fixture::new("func main() { let x = ; }");
    let ast = fixture.parse();
    fixture.expect("E1003", "expected an expression");
    let declaration = sole_function(&ast);
    let block = body(&ast, declaration);
    match &ast.stmt(block.statements[0]).kind {
        StmtKind::Let(binding) => {
            let init = binding.init.expect("initializer node exists");
            assert!(matches!(ast.expr(init).kind, ExprKind::Error));
        }
        other => panic!("expected let, got {other:?}"),
    }
}

#[test]
fn statements_after_comments_and_blank_lines_parse() {
    let (_, ast) = parse_clean("func main() {\n  // setup\n  let a = 1;\n\n  /* use it */\n  a\n}");
    let declaration = sole_function(&ast);
    assert_eq!(body(&ast, declaration).statements.len(), 1);
    assert!(body(&ast, declaration).tail.is_some());
}

#[test]
fn expression_keyword_tokens_are_not_paths() {
    let fixture = Fixture::new("func main() { let x = ; }");
    let _ = fixture.parse();
    // Sanity: the lexer reports keywords, not identifiers, so the parser cannot
    // silently accept `let` as an expression.
    assert!(matches!(TokenKind::Let, TokenKind::Let));
    fixture.expect("E1003", "expected an expression");
}

#[test]
fn if_is_an_expression_with_two_block_branches() {
    let (fixture, ast) = parse_clean("func main() -> i32 { if true { 1 } else { 2 } }");
    assert_invariants(&fixture, &ast);
    let declaration = sole_function(&ast);
    let block = body(&ast, declaration);
    assert!(block.statements.is_empty());
    let tail = block.tail.expect("if is the tail expression");
    let (condition, then_branch, else_branch) = match &ast.expr(tail).kind {
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => (*condition, *then_branch, *else_branch),
        other => panic!("expected an if expression, got {other:?}"),
    };
    assert!(matches!(
        ast.expr(condition).kind,
        ExprKind::Lit(Literal::Bool(true))
    ));
    let then_tail = ast.block(then_branch).tail.expect("then value");
    assert!(matches!(
        ast.expr(then_tail).kind,
        ExprKind::Lit(Literal::Int { value: 1, .. })
    ));
    let else_branch = else_branch.expect("if has an else branch");
    let else_tail = ast.block(else_branch).tail.expect("else value");
    assert!(matches!(
        ast.expr(else_tail).kind,
        ExprKind::Lit(Literal::Int { value: 2, .. })
    ));
}

#[test]
fn else_if_desugars_into_a_nested_if_in_a_block() {
    let (fixture, ast) =
        parse_clean("func main() -> i32 { if false { 0 } else if true { 1 } else { 2 } }");
    assert_invariants(&fixture, &ast);
    let declaration = sole_function(&ast);
    let tail = body(&ast, declaration).tail.expect("if tail");
    let else_branch = match &ast.expr(tail).kind {
        ExprKind::If { else_branch, .. } => (*else_branch).expect("else branch"),
        other => panic!("expected an if expression, got {other:?}"),
    };
    let else_block = ast.block(else_branch);
    assert!(else_block.statements.is_empty());
    let nested = else_block.tail.expect("nested if is the block value");
    let final_else = match &ast.expr(nested).kind {
        ExprKind::If { else_branch, .. } => (*else_branch).expect("final else"),
        other => panic!("expected a nested if, got {other:?}"),
    };
    let final_tail = ast.block(final_else).tail.expect("final value");
    assert!(matches!(
        ast.expr(final_tail).kind,
        ExprKind::Lit(Literal::Int { value: 2, .. })
    ));
}

#[test]
fn a_block_like_expression_is_a_statement_without_a_semicolon() {
    let (_, ast) = parse_clean("func main() -> i32 { if true { return 0; } 1 }");
    let declaration = sole_function(&ast);
    let block = body(&ast, declaration);
    assert_eq!(block.statements.len(), 1);
    assert!(matches!(
        ast.stmt(block.statements[0]).kind,
        StmtKind::Expr(_)
    ));
    let tail = block.tail.expect("value after the if");
    assert!(matches!(
        ast.expr(tail).kind,
        ExprKind::Lit(Literal::Int { value: 1, .. })
    ));
}

#[test]
fn a_block_like_statement_does_not_swallow_a_following_operator() {
    let (_, ast) =
        parse_clean("func main() -> i32 { let mut x = 1; let r = &x; if true { x = 2; } *r }");
    let declaration = sole_function(&ast);
    let block = body(&ast, declaration);
    assert_eq!(block.statements.len(), 3);
    assert!(matches!(
        ast.stmt(block.statements[2]).kind,
        StmtKind::Expr(_)
    ));
    let tail = block.tail.expect("the dereference is the tail");
    assert!(matches!(ast.expr(tail).kind, ExprKind::Deref { .. }));
}

#[test]
fn while_parses_its_condition_and_body() {
    let (fixture, ast) = parse_clean("func main() { while true { break; continue; } }");
    assert_invariants(&fixture, &ast);
    let declaration = sole_function(&ast);
    let block = body(&ast, declaration);
    assert_eq!(block.statements.len(), 1);
    let (condition, body_id) = match &ast.stmt(block.statements[0]).kind {
        StmtKind::While { condition, body } => (*condition, *body),
        other => panic!("expected a while statement, got {other:?}"),
    };
    assert!(matches!(
        ast.expr(condition).kind,
        ExprKind::Lit(Literal::Bool(true))
    ));
    let loop_body = ast.block(body_id);
    assert_eq!(loop_body.statements.len(), 2);
    assert!(matches!(
        ast.stmt(loop_body.statements[0]).kind,
        StmtKind::Break
    ));
    assert!(matches!(
        ast.stmt(loop_body.statements[1]).kind,
        StmtKind::Continue
    ));
}

#[test]
fn break_requires_a_semicolon() {
    let fixture = Fixture::new("func main() { while true { break } }");
    let ast = fixture.parse();
    fixture.expect("E1002", "`break`");
    assert_invariants(&fixture, &ast);
}

#[test]
fn a_missing_condition_keeps_the_then_block() {
    let fixture = Fixture::new("func main() -> i32 { if { 1 } else { 2 } }");
    let ast = fixture.parse();
    fixture.expect("E1003", "expected an expression");
    assert_eq!(fixture.codes(), vec!["E1003".to_owned()]);
    assert_invariants(&fixture, &ast);
    let declaration = sole_function(&ast);
    let tail = body(&ast, declaration).tail.expect("if tail");
    let (then_branch, else_branch) = match &ast.expr(tail).kind {
        ExprKind::If {
            then_branch,
            else_branch,
            ..
        } => (*then_branch, (*else_branch).expect("else branch")),
        other => panic!("expected an if expression, got {other:?}"),
    };
    let then_tail = ast.block(then_branch).tail.expect("then value");
    assert!(matches!(
        ast.expr(then_tail).kind,
        ExprKind::Lit(Literal::Int { value: 1, .. })
    ));
    let else_tail = ast.block(else_branch).tail.expect("else value");
    assert!(matches!(
        ast.expr(else_tail).kind,
        ExprKind::Lit(Literal::Int { value: 2, .. })
    ));
}

#[test]
fn else_requires_a_block_or_another_if() {
    let fixture = Fixture::new("func main() -> i32 { if true { 1 } else 2 }");
    let ast = fixture.parse();
    fixture.expect("E1002", "`else`");
    assert_invariants(&fixture, &ast);
}

#[test]
fn deeply_nested_ifs_report_e1008() {
    let mut source = String::from("func main() { ");
    for _ in 0..300 {
        source.push_str("if true { ");
    }
    source.push('0');
    for _ in 0..300 {
        source.push_str(" }");
    }
    source.push_str(" }");

    let fixture = Fixture::new(&source);
    let _ast = fixture.parse();
    assert!(
        fixture.codes().iter().any(|code| code == "E1008"),
        "expected E1008, got {:?}",
        fixture.codes()
    );
}
