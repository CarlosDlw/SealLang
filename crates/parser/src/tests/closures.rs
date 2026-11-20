//! Closure expression syntax.

use seal_ast::{
    Ast, BinaryOp, ClosureParam, ExprId, ExprKind, FnDecl, Literal, PrimitiveType, TypeId, TypeKind,
};

use super::support::{Fixture, assert_invariants, parse_clean, sole_function, tail};

/// Returns the closure in the tail expression, with its parse data.
fn tail_closure<'a>(
    ast: &'a Ast,
    declaration: &FnDecl,
) -> (bool, &'a [ClosureParam], Option<TypeId>, ExprId) {
    match &tail(ast, declaration).kind {
        ExprKind::Closure {
            is_move,
            params,
            ret,
            body,
        } => (*is_move, params, *ret, *body),
        other => panic!("expected a closure, got {other:?}"),
    }
}

#[test]
fn a_unit_closure_parses_without_parameters() {
    let (fixture, ast) = parse_clean("func main() -> i32 { || 1 }");
    assert_invariants(&fixture, &ast);

    let declaration = sole_function(&ast);
    let (is_move, params, ret, body) = tail_closure(&ast, declaration);
    assert!(!is_move);
    assert!(params.is_empty());
    assert!(ret.is_none());
    assert!(matches!(
        ast.expr(body).kind,
        ExprKind::Lit(Literal::Int { .. })
    ));
}

#[test]
fn closure_parameters_accept_optional_types() {
    let (fixture, ast) = parse_clean("func main() -> i32 { |x: i32, y| x + y }");
    assert_invariants(&fixture, &ast);

    let declaration = sole_function(&ast);
    let (is_move, params, ret, body) = tail_closure(&ast, declaration);
    assert!(!is_move);
    assert!(ret.is_none());
    assert_eq!(params.len(), 2);
    assert_eq!(fixture.symbol(params[0].name), "x");
    assert!(matches!(
        ast.ty(params[0].ty.expect("the first parameter is annotated"))
            .kind,
        TypeKind::Primitive(PrimitiveType::I32)
    ));
    assert_eq!(fixture.symbol(params[1].name), "y");
    assert!(params[1].ty.is_none());
    assert!(matches!(
        ast.expr(body).kind,
        ExprKind::Binary {
            op: BinaryOp::Add,
            ..
        }
    ));
}

#[test]
fn a_trailing_comma_is_allowed_in_the_parameter_list() {
    let (fixture, ast) = parse_clean("func main() -> i32 { |x,| x }");
    assert_invariants(&fixture, &ast);

    let declaration = sole_function(&ast);
    let (_, params, _, _) = tail_closure(&ast, declaration);
    assert_eq!(params.len(), 1);
    assert_eq!(fixture.symbol(params[0].name), "x");
}

#[test]
fn move_is_recorded_and_keeps_the_parameter_list() {
    let (fixture, ast) = parse_clean("func main() -> i32 { move |x| x }");
    assert_invariants(&fixture, &ast);

    let declaration = sole_function(&ast);
    let (is_move, params, _, _) = tail_closure(&ast, declaration);
    assert!(is_move);
    assert_eq!(params.len(), 1);
    assert_eq!(
        fixture.text(tail(&ast, declaration).span),
        "move |x| x",
        "the closure span starts at `move` and ends at the body"
    );

    let (fixture, ast) = parse_clean("func main() -> i32 { move || 1 }");
    assert_invariants(&fixture, &ast);
    let declaration = sole_function(&ast);
    let (is_move, params, _, _) = tail_closure(&ast, declaration);
    assert!(is_move);
    assert!(params.is_empty());
}

#[test]
fn an_explicit_result_type_requires_a_block_body() {
    let (fixture, ast) = parse_clean("func main() -> i32 { |x: i32| -> i32 { x } }");
    assert_invariants(&fixture, &ast);

    let declaration = sole_function(&ast);
    let (_, params, ret, body) = tail_closure(&ast, declaration);
    assert_eq!(params.len(), 1);
    let ret = ret.expect("the closure declares a result type");
    assert!(matches!(
        ast.ty(ret).kind,
        TypeKind::Primitive(PrimitiveType::I32)
    ));
    assert!(matches!(ast.expr(body).kind, ExprKind::Block(_)));
}

#[test]
fn an_explicit_result_type_without_a_block_is_rejected() {
    let fixture = Fixture::new("func main() -> i32 { |x| -> i32 x }");
    let ast = fixture.parse();
    fixture.expect("E1002", "needs a block body");
    assert_eq!(fixture.codes(), ["E1002"], "one diagnostic, no cascade");

    let declaration = sole_function(&ast);
    let (_, _, ret, _) = tail_closure(&ast, declaration);
    assert!(ret.is_some(), "recovery keeps the declared result type");
}

#[test]
fn a_discarded_parameter_is_allowed() {
    let (fixture, ast) = parse_clean("func main() -> i32 { |_| 0 }");
    assert_invariants(&fixture, &ast);

    let declaration = sole_function(&ast);
    let (_, params, _, _) = tail_closure(&ast, declaration);
    assert_eq!(params.len(), 1);
    assert_eq!(fixture.symbol(params[0].name), "_");
}

#[test]
fn move_unit_closures_nest() {
    let (fixture, ast) = parse_clean("func main() -> i32 { move || || 1 }");
    assert_invariants(&fixture, &ast);

    let declaration = sole_function(&ast);
    let (is_move, params, _, outer_body) = tail_closure(&ast, declaration);
    assert!(is_move);
    assert!(params.is_empty());
    assert!(matches!(
        &ast.expr(outer_body).kind,
        ExprKind::Closure {
            is_move: false,
            params,
            ..
        } if params.is_empty()
    ));
}

#[test]
fn a_pipe_after_the_body_is_still_bit_or() {
    let (fixture, ast) = parse_clean("func main() -> i32 { |x| x | 1 }");
    assert_invariants(&fixture, &ast);

    let declaration = sole_function(&ast);
    let (_, _, _, body) = tail_closure(&ast, declaration);
    let ExprKind::Binary {
        op: BinaryOp::BitOr,
        lhs,
        rhs,
    } = ast.expr(body).kind
    else {
        panic!("expected the body to be a bitwise or");
    };
    assert!(matches!(ast.expr(lhs).kind, ExprKind::Path { .. }));
    assert!(matches!(
        ast.expr(rhs).kind,
        ExprKind::Lit(Literal::Int { .. })
    ));
}

#[test]
fn closures_nest_in_the_body_of_another_closure() {
    let (fixture, ast) = parse_clean("func main() -> i32 { |x| |y| x + y }");
    assert_invariants(&fixture, &ast);

    let declaration = sole_function(&ast);
    let (_, outer_params, _, outer_body) = tail_closure(&ast, declaration);
    assert_eq!(outer_params.len(), 1);
    let ExprKind::Closure {
        params: inner_params,
        body: inner_body,
        ..
    } = &ast.expr(outer_body).kind
    else {
        panic!("expected the outer body to be a closure");
    };
    assert_eq!(inner_params.len(), 1);
    assert!(matches!(
        ast.expr(*inner_body).kind,
        ExprKind::Binary {
            op: BinaryOp::Add,
            ..
        }
    ));
}

#[test]
fn a_closure_body_uses_full_expression_precedence() {
    let (fixture, ast) = parse_clean("func main() -> i32 { |x| x * 2 + 1 }");
    assert_invariants(&fixture, &ast);

    let declaration = sole_function(&ast);
    let (_, _, _, body) = tail_closure(&ast, declaration);
    let ExprKind::Binary {
        op: BinaryOp::Add,
        lhs,
        ..
    } = ast.expr(body).kind
    else {
        panic!("expected an addition at the top of the body");
    };
    assert!(matches!(
        ast.expr(lhs).kind,
        ExprKind::Binary {
            op: BinaryOp::Mul,
            ..
        }
    ));
}

#[test]
fn move_outside_a_closure_is_an_error() {
    let fixture = Fixture::new("func main() -> i32 { move }");
    let ast = fixture.parse();
    fixture.expect("E1002", "expected `|` after `move`");

    let declaration = sole_function(&ast);
    assert!(matches!(tail(&ast, declaration).kind, ExprKind::Error));
}

#[test]
fn or_or_between_expressions_stays_logical_or() {
    let (fixture, ast) = parse_clean("func main() -> bool { ready || failed }");
    assert_invariants(&fixture, &ast);

    let declaration = sole_function(&ast);
    assert!(matches!(
        tail(&ast, declaration).kind,
        ExprKind::Binary {
            op: BinaryOp::Or,
            ..
        }
    ));
}

#[test]
fn an_empty_parameter_list_with_a_result_type_is_a_closure() {
    let (fixture, ast) = parse_clean("func main() -> i32 { || -> i32 { 0 } }");
    assert_invariants(&fixture, &ast);

    let declaration = sole_function(&ast);
    let (is_move, params, ret, body) = tail_closure(&ast, declaration);
    assert!(!is_move);
    assert!(params.is_empty());
    assert!(ret.is_some());
    assert!(matches!(ast.expr(body).kind, ExprKind::Block(_)));
}
