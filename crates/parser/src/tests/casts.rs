//! Cast expressions: precedence, associativity, targets, and recovery.

use seal_ast::{Expr, ExprKind, TypeKind, UnaryOp};

use crate::tests::support::{assert_invariants, parse_clean, sole_function};

fn tail_expr(
    source: &str,
) -> (
    crate::tests::support::Fixture,
    seal_ast::Ast,
    seal_ast::ExprId,
) {
    let (fixture, ast) = parse_clean(source);
    assert_invariants(&fixture, &ast);
    let declaration = sole_function(&ast);
    let id = ast.block(declaration.body).tail.expect("tail exists");
    (fixture, ast, id)
}

fn cast<'a>(ast: &'a seal_ast::Ast, expr: &'a Expr) -> (&'a Expr, seal_ast::TypeId) {
    match &expr.kind {
        ExprKind::Cast { operand, ty, .. } => (ast.expr(*operand), *ty),
        other => panic!("expected a cast, got {other:?}"),
    }
}

#[test]
fn a_cast_keeps_its_target_type() {
    let (_, ast, id) = tail_expr("func main(value: i32) { value as *mut u8 }");
    let (operand, ty) = cast(&ast, ast.expr(id));
    assert!(matches!(operand.kind, ExprKind::Path { .. }));
    match ast.ty(ty).kind {
        TypeKind::Pointer { mutable, .. } => assert!(mutable),
        ref other => panic!("expected a pointer type, got {other:?}"),
    }
}

#[test]
fn casts_are_left_associative() {
    let (_, ast, id) = tail_expr("func main(value: i32) { value as u16 as i32 }");
    let (inner, _) = cast(&ast, ast.expr(id));
    let (operand, _) = cast(&ast, inner);
    assert!(matches!(operand.kind, ExprKind::Path { .. }));
}

#[test]
fn a_prefix_operator_binds_before_a_cast() {
    let (_, ast, id) = tail_expr("func main(value: i32) { -value as i32 }");
    let (operand, _) = cast(&ast, ast.expr(id));
    match &operand.kind {
        ExprKind::Unary { op, .. } => assert_eq!(*op, UnaryOp::Neg),
        other => panic!("expected unary negation, got {other:?}"),
    }
}

#[test]
fn a_cast_binds_tighter_than_multiplication() {
    let (_, ast, id) = tail_expr("func main(value: i32, other: i32) { value * other as i32 }");
    let ExprKind::Binary { op, rhs, .. } = &ast.expr(id).kind else {
        panic!("expected multiplication, got {:?}", ast.expr(id).kind);
    };
    assert_eq!(*op, seal_ast::BinaryOp::Mul);
    let (operand, _) = cast(&ast, ast.expr(*rhs));
    assert!(matches!(operand.kind, ExprKind::Path { .. }));
}

#[test]
fn a_cast_on_the_left_is_grouped_before_multiplication() {
    let (_, ast, id) = tail_expr("func main(value: i32, other: i32) { value as i32 * other }");
    let ExprKind::Binary { op, lhs, .. } = &ast.expr(id).kind else {
        panic!("expected multiplication, got {:?}", ast.expr(id).kind);
    };
    assert_eq!(*op, seal_ast::BinaryOp::Mul);
    let (operand, _) = cast(&ast, ast.expr(*lhs));
    assert!(matches!(operand.kind, ExprKind::Path { .. }));
}

#[test]
fn a_missing_cast_target_recovers_at_the_next_statement() {
    let fixture = crate::tests::support::Fixture::new("func main(value: i32) { value as; 0 }");
    let _ = fixture.parse();
    fixture.expect("E1004", "expected a type, found `;`");
}

#[test]
fn as_is_a_keyword_and_not_a_binding_name() {
    let fixture = crate::tests::support::Fixture::new("func main() { let as = 1; 0 }");
    let _ = fixture.parse();
    assert!(
        !fixture.diagnostics().is_empty(),
        "`as` must not parse as an identifier"
    );
}
