//! Method-call syntax: receivers, arguments, chaining, and field access.

use seal_ast::ExprKind;

use super::support::{body, parse_clean, sole_function, tail};

#[test]
fn a_method_call_is_distinct_from_a_field_access() {
    let (fixture, ast) = parse_clean("func f(a: [i32; 3]) -> usize { a.len() }");
    let declaration = sole_function(&ast);
    let ExprKind::MethodCall {
        receiver,
        name,
        args,
        ..
    } = tail(&ast, declaration).kind.clone()
    else {
        panic!("expected a method call");
    };
    assert_eq!(fixture.symbol(name), "len");
    assert!(args.is_empty());
    assert!(matches!(ast.expr(receiver).kind, ExprKind::Path(_)));

    let (_, ast) = parse_clean("func f(p: P) -> i32 { p.x }");
    let declaration = sole_function(&ast);
    assert!(matches!(
        tail(&ast, declaration).kind,
        ExprKind::Field { .. }
    ));
}

#[test]
fn method_arguments_are_parsed_in_source_order() {
    let (fixture, ast) = parse_clean("func f(a: [i32; 3]) { a.swap(0, 2); }");
    let declaration = sole_function(&ast);
    let statement = body(&ast, declaration).statements[0];
    let seal_ast::StmtKind::Expr(expr) = ast.stmt(statement).kind.clone() else {
        panic!("expected an expression statement");
    };
    let ExprKind::MethodCall { name, args, .. } = ast.expr(expr).kind.clone() else {
        panic!("expected a method call");
    };
    assert_eq!(fixture.symbol(name), "swap");
    assert_eq!(args.len(), 2);
}

#[test]
fn method_calls_chain_from_left_to_right() {
    let (fixture, ast) = parse_clean("func f() -> i32 { (-5).abs().min(10) }");
    let declaration = sole_function(&ast);
    let ExprKind::MethodCall {
        receiver,
        name,
        args,
        ..
    } = tail(&ast, declaration).kind.clone()
    else {
        panic!("expected the outer method call");
    };
    assert_eq!(fixture.symbol(name), "min");
    assert_eq!(args.len(), 1);
    assert!(matches!(
        ast.expr(receiver).kind,
        ExprKind::MethodCall { .. }
    ));
}
