//! Expression grammar: precedence, associativity, calls, updates, literals.

use seal_ast::{AssignOp, BinaryOp, Expr, ExprKind, Literal, UnaryOp, UpdateOp};

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

fn binary<'a>(ast: &'a seal_ast::Ast, expr: &'a Expr) -> (BinaryOp, &'a Expr, &'a Expr) {
    match &expr.kind {
        ExprKind::Binary { op, lhs, rhs } => (*op, ast.expr(*lhs), ast.expr(*rhs)),
        other => panic!("expected a binary expression, got {other:?}"),
    }
}

fn assign<'a>(ast: &'a seal_ast::Ast, expr: &'a Expr) -> (AssignOp, &'a Expr, &'a Expr) {
    match &expr.kind {
        ExprKind::Assign { op, target, value } => (*op, ast.expr(*target), ast.expr(*value)),
        other => panic!("expected an assignment, got {other:?}"),
    }
}

fn int(expr: &Expr) -> (u128, u32, Option<seal_symbol::Symbol>) {
    match expr.kind {
        ExprKind::Lit(Literal::Int {
            value,
            base,
            suffix,
        }) => (value, base, suffix),
        ref other => panic!("expected an integer literal, got {other:?}"),
    }
}

fn path(expr: &Expr) -> seal_symbol::Symbol {
    match expr.kind {
        ExprKind::Path { name, .. } => name,
        ref other => panic!("expected a path, got {other:?}"),
    }
}

#[test]
fn exclusive_borrows_parse() {
    let (_, ast, id) = tail_expr("func main(a: i32) { &mut a }");
    match &ast.expr(id).kind {
        ExprKind::Borrow { operand, mutable } => {
            assert!(mutable);
            assert!(matches!(ast.expr(*operand).kind, ExprKind::Path { .. }));
        }
        other => panic!("expected an exclusive borrow, got {other:?}"),
    }
}

#[test]
fn sharing_is_the_default_borrow() {
    let (_, ast, id) = tail_expr("func main(a: i32) { &a }");
    match &ast.expr(id).kind {
        ExprKind::Borrow { mutable, .. } => assert!(!mutable),
        other => panic!("expected a borrow, got {other:?}"),
    }
}

#[test]
fn exclusive_borrows_can_target_a_dereference() {
    let (_, ast, id) = tail_expr("func main(p: *mut i32) { &mut *p }");
    match &ast.expr(id).kind {
        ExprKind::Borrow { operand, mutable } => {
            assert!(mutable);
            assert!(matches!(ast.expr(*operand).kind, ExprKind::Deref { .. }));
        }
        other => panic!("expected an exclusive borrow, got {other:?}"),
    }
}

#[test]
fn literals_keep_their_payloads() {
    let (fixture, ast, id) = tail_expr("func main() { 0xFFu8 }");
    let (value, base, suffix) = int(ast.expr(id));
    assert_eq!((value, base), (255, 16));
    assert_eq!(fixture.symbol(suffix.expect("suffix")), "u8");

    let (_, ast, id) = tail_expr("func main() { 1.5f32 }");
    match ast.expr(id).kind {
        ExprKind::Lit(Literal::Float { value, suffix }) => {
            assert_eq!(value, 1.5);
            assert!(suffix.is_some());
        }
        ref other => panic!("expected float, got {other:?}"),
    }

    let (fixture, ast, id) = tail_expr("func main() { \"hi\" }");
    match ast.expr(id).kind {
        ExprKind::Lit(Literal::Str(symbol)) => assert_eq!(fixture.symbol(symbol), "hi"),
        ref other => panic!("expected string, got {other:?}"),
    }

    let (fixture, ast, id) = tail_expr("func main() { c\"hi\" }");
    match ast.expr(id).kind {
        ExprKind::Lit(Literal::CStr(symbol)) => assert_eq!(fixture.symbol(symbol), "hi"),
        ref other => panic!("expected C string, got {other:?}"),
    }

    let (_, ast, id) = tail_expr("func main() { 'x' }");
    assert!(matches!(
        ast.expr(id).kind,
        ExprKind::Lit(Literal::Char('x'))
    ));

    let (_, ast, id) = tail_expr("func main() { true }");
    assert!(matches!(
        ast.expr(id).kind,
        ExprKind::Lit(Literal::Bool(true))
    ));
}

#[test]
fn multiplication_binds_tighter_than_addition() {
    let (_, ast, id) = tail_expr("func main() { 1 + 2 * 3 }");
    let (op, lhs, rhs) = binary(&ast, ast.expr(id));
    assert_eq!(op, BinaryOp::Add);
    assert_eq!(int(lhs).0, 1);
    let (inner_op, inner_lhs, inner_rhs) = binary(&ast, rhs);
    assert_eq!(inner_op, BinaryOp::Mul);
    assert_eq!(int(inner_lhs).0, 2);
    assert_eq!(int(inner_rhs).0, 3);

    let (_, ast, id) = tail_expr("func main() { 2 * 3 + 4 }");
    let (op, lhs, rhs) = binary(&ast, ast.expr(id));
    assert_eq!(op, BinaryOp::Add);
    assert_eq!(int(rhs).0, 4);
    assert_eq!(binary(&ast, lhs).0, BinaryOp::Mul);
}

#[test]
fn subtraction_is_left_associative() {
    let (_, ast, id) = tail_expr("func main() { 1 - 2 - 3 }");
    let (op, lhs, rhs) = binary(&ast, ast.expr(id));
    assert_eq!(op, BinaryOp::Sub);
    assert_eq!(int(rhs).0, 3);
    let (inner_op, inner_lhs, inner_rhs) = binary(&ast, lhs);
    assert_eq!(inner_op, BinaryOp::Sub);
    assert_eq!(int(inner_lhs).0, 1);
    assert_eq!(int(inner_rhs).0, 2);
}

#[test]
fn assignment_is_right_associative() {
    let (fixture, ast, id) = tail_expr("func main() { a = b = c }");
    let (op, target, value) = assign(&ast, ast.expr(id));
    assert_eq!(op, AssignOp::Assign);
    assert_eq!(fixture.symbol(path(target)), "a");
    let (inner_op, inner_target, inner_value) = assign(&ast, value);
    assert_eq!(inner_op, AssignOp::Assign);
    assert_eq!(fixture.symbol(path(inner_target)), "b");
    assert_eq!(fixture.symbol(path(inner_value)), "c");
}

#[test]
fn bitwise_shifts_and_logic_follow_c_precedence() {
    let (_, ast, id) = tail_expr("func main() { 1 << 2 + 3 }");
    let (op, lhs, rhs) = binary(&ast, ast.expr(id));
    assert_eq!(op, BinaryOp::Shl);
    assert_eq!(int(lhs).0, 1);
    assert_eq!(binary(&ast, rhs).0, BinaryOp::Add);

    let (_, ast, id) = tail_expr("func main() { 1 | 2 & 3 }");
    let (op, lhs, rhs) = binary(&ast, ast.expr(id));
    assert_eq!(op, BinaryOp::BitOr);
    assert_eq!(int(lhs).0, 1);
    assert_eq!(binary(&ast, rhs).0, BinaryOp::BitAnd);

    let (_, ast, id) = tail_expr("func main() { true || false && true }");
    let (op, lhs, rhs) = binary(&ast, ast.expr(id));
    assert_eq!(op, BinaryOp::Or);
    assert!(matches!(lhs.kind, ExprKind::Lit(Literal::Bool(true))));
    assert_eq!(binary(&ast, rhs).0, BinaryOp::And);

    let (_, ast, id) = tail_expr("func main() { 1 == 2 < 3 }");
    let (op, _, rhs) = binary(&ast, ast.expr(id));
    assert_eq!(op, BinaryOp::Eq);
    assert_eq!(binary(&ast, rhs).0, BinaryOp::Lt);
}

#[test]
fn prefix_operators_bind_above_binary_operators() {
    let (_, ast, id) = tail_expr("func main() { -2 * 3 }");
    let (op, lhs, rhs) = binary(&ast, ast.expr(id));
    assert_eq!(op, BinaryOp::Mul);
    assert_eq!(int(rhs).0, 3);
    match lhs.kind {
        ExprKind::Unary { op, operand } => {
            assert_eq!(op, UnaryOp::Neg);
            assert_eq!(int(ast.expr(operand)).0, 2);
        }
        ref other => panic!("expected unary, got {other:?}"),
    }

    let (_, ast, id) = tail_expr("func main() { !a && b }");
    let (op, lhs, _) = binary(&ast, ast.expr(id));
    assert_eq!(op, BinaryOp::And);
    assert!(matches!(
        lhs.kind,
        ExprKind::Unary {
            op: UnaryOp::Not,
            ..
        }
    ));

    let (_, ast, id) = tail_expr("func main() { ~1 }");
    assert!(matches!(
        ast.expr(id).kind,
        ExprKind::Unary {
            op: UnaryOp::BitNot,
            ..
        }
    ));
}

#[test]
fn borrows_and_derefs_bind_like_prefix_operators() {
    let (_, ast, id) = tail_expr("func main() { *&a }");
    match ast.expr(id).kind {
        ExprKind::Deref { operand } => {
            assert!(matches!(ast.expr(operand).kind, ExprKind::Borrow { .. }));
        }
        ref other => panic!("expected a dereference, got {other:?}"),
    }

    let (fixture, ast, id) = tail_expr("func main() { &a == b }");
    let (op, lhs, rhs) = binary(&ast, ast.expr(id));
    assert_eq!(op, BinaryOp::Eq);
    assert!(matches!(lhs.kind, ExprKind::Borrow { .. }));
    assert_eq!(fixture.symbol(path(rhs)), "b");

    let (_, ast, id) = tail_expr("func main() { &*p + 1 }");
    let (op, lhs, rhs) = binary(&ast, ast.expr(id));
    assert_eq!(op, BinaryOp::Add);
    assert_eq!(int(rhs).0, 1);
    match lhs.kind {
        ExprKind::Borrow { operand, .. } => {
            assert!(matches!(ast.expr(operand).kind, ExprKind::Deref { .. }));
        }
        ref other => panic!("expected a borrow, got {other:?}"),
    }
}

#[test]
fn updates_distinguish_prefix_and_postfix() {
    let (fixture, ast, id) = tail_expr("func main() { a++ + ++b }");
    let (op, lhs, rhs) = binary(&ast, ast.expr(id));
    assert_eq!(op, BinaryOp::Add);
    match lhs.kind {
        ExprKind::Update {
            op: UpdateOp::Increment,
            operand,
            postfix,
        } => {
            assert!(postfix);
            assert_eq!(fixture.symbol(path(ast.expr(operand))), "a");
        }
        ref other => panic!("expected postfix update, got {other:?}"),
    }
    match rhs.kind {
        ExprKind::Update {
            op: UpdateOp::Increment,
            operand,
            postfix,
        } => {
            assert!(!postfix);
            assert_eq!(fixture.symbol(path(ast.expr(operand))), "b");
        }
        ref other => panic!("expected prefix update, got {other:?}"),
    }
}

#[test]
fn calls_accept_trailing_commas_and_chain() {
    let (fixture, ast, id) = tail_expr("func main() { f(1, 2,) }");
    match &ast.expr(id).kind {
        ExprKind::Call { callee, args } => {
            assert_eq!(fixture.symbol(path(ast.expr(*callee))), "f");
            assert_eq!(args.len(), 2);
            assert_eq!(int(ast.expr(args[0])).0, 1);
            assert_eq!(int(ast.expr(args[1])).0, 2);
        }
        other => panic!("expected call, got {other:?}"),
    }

    let (_, ast, id) = tail_expr("func main() { f(1)(2) }");
    match &ast.expr(id).kind {
        ExprKind::Call { callee, args } => {
            assert_eq!(args.len(), 1);
            assert!(matches!(ast.expr(*callee).kind, ExprKind::Call { .. }));
        }
        other => panic!("expected outer call, got {other:?}"),
    }
}

#[test]
fn parentheses_group_and_keep_their_span() {
    let (fixture, ast, id) = tail_expr("func main() { (1 + 2) * 3 }");
    let (op, lhs, rhs) = binary(&ast, ast.expr(id));
    assert_eq!(op, BinaryOp::Mul);
    assert_eq!(int(rhs).0, 3);
    match &lhs.kind {
        ExprKind::Paren(inner) => {
            assert_eq!(binary(&ast, ast.expr(*inner)).0, BinaryOp::Add);
            assert_eq!(fixture.text(lhs.span), "(1 + 2)");
        }
        other => panic!("expected paren, got {other:?}"),
    }
}

#[test]
fn every_compound_assignment_is_mapped() {
    let cases = [
        ("+=", AssignOp::AddAssign),
        ("-=", AssignOp::SubAssign),
        ("*=", AssignOp::MulAssign),
        ("/=", AssignOp::DivAssign),
        ("%=", AssignOp::RemAssign),
        ("<<=", AssignOp::ShlAssign),
        (">>=", AssignOp::ShrAssign),
        ("&=", AssignOp::BitAndAssign),
        ("|=", AssignOp::BitOrAssign),
        ("^=", AssignOp::BitXorAssign),
    ];
    for (spelling, expected) in cases {
        let source = format!("func main() {{ a {spelling} 1 }}");
        let (_, ast) = parse_clean(&source);
        let declaration = sole_function(&ast);
        let id = ast.block(declaration.body).tail.expect("tail exists");
        let (op, target, value) = assign(&ast, ast.expr(id));
        assert_eq!(op, expected, "operator {spelling}");
        assert!(matches!(target.kind, ExprKind::Path { .. }));
        assert_eq!(int(value).0, 1);
    }
}

#[test]
fn invalid_assignment_targets_report_e1006() {
    for source in [
        "func main() { 1 = 2 }",
        "func main() { (1 + 2) = 3 }",
        "func main() { 1++ }",
        "func main() { ++1 }",
    ] {
        let fixture = crate::tests::support::Fixture::new(source);
        let _ = fixture.parse();
        fixture.expect("E1006", "invalid");
    }
}

#[test]
fn a_missing_comma_between_arguments_is_diagnosed() {
    let fixture = crate::tests::support::Fixture::new("func main() { f(1 2) }");
    let ast = fixture.parse();
    fixture.expect("E1002", "argument list");
    let declaration = sole_function(&ast);
    let id = ast.block(declaration.body).tail.expect("tail exists");
    match &ast.expr(id).kind {
        ExprKind::Call { args, .. } => assert_eq!(args.len(), 2),
        other => panic!("expected call, got {other:?}"),
    }
}

#[test]
fn missing_closing_parenthesis_is_diagnosed() {
    let fixture = crate::tests::support::Fixture::new("func main() { f(1 }");
    let _ = fixture.parse();
    fixture.expect("E1002", "`)`");
}
