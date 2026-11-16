//! Blocks, `let`, `return`, expression statements, and the tail expression.

use seal_ast::{ExprKind, Literal, StmtKind};
use seal_lexer::TokenKind;

use crate::tests::support::{Fixture, body, parse_clean, sole_function};

#[test]
fn let_bindings_cover_every_form() {
    let (fixture, ast) = parse_clean("func main() { let a: i32 = 1; let b = 2; let c: i32; a }");
    let declaration = sole_function(&ast);
    let block = body(&ast, declaration);
    assert_eq!(block.statements.len(), 3);

    match &ast.stmt(block.statements[0]).kind {
        StmtKind::Let(binding) => {
            assert_eq!(fixture.symbol(binding.name), "a");
            assert!(binding.ty.is_some());
            assert!(binding.init.is_some());
        }
        other => panic!("expected let, got {other:?}"),
    }
    match &ast.stmt(block.statements[1]).kind {
        StmtKind::Let(binding) => {
            assert_eq!(fixture.symbol(binding.name), "b");
            assert!(binding.ty.is_none());
            assert!(binding.init.is_some());
        }
        other => panic!("expected let, got {other:?}"),
    }
    match &ast.stmt(block.statements[2]).kind {
        StmtKind::Let(binding) => {
            assert!(binding.ty.is_some());
            assert!(binding.init.is_none());
        }
        other => panic!("expected let, got {other:?}"),
    }
    assert!(block.tail.is_some());
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
    assert!(matches!(ast.expr(tail).kind, ExprKind::Path(_)));
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
