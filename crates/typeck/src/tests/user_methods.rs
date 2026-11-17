//! User-defined method and associated-function checking.

use seal_hir::{ExprKind, ReceiverKind};

use crate::{MethodKind, ReceiverAdjust};

use super::support::{check_clean, check_errors, function, statements, tail};

const POINT: &str = "\
struct Point { x: i32, y: i32 } \
impl Point { \
    func new(x: i32, y: i32) -> Point { Point { x: x, y: y } } \
    func sum(self: *Point) -> i32 { (*self).x + (*self).y } \
    func shift(self: *mut Point, dx: i32, dy: i32) { (*self).x += dx; (*self).y += dy; } \
    func into_x(self: Point) -> i32 { self.x } \
}";

#[test]
fn user_methods_resolve_with_the_declared_receiver() {
    let source = format!(
        "{POINT} func main() -> i32 {{ \
            let mut point = Point::new(1, 2); \
            point.shift(3, 4); \
            point.sum() \
        }}"
    );
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "main");
    let shift = method_expr(&hir, statements(&hir, body)[1]);
    let resolved = results.method(shift).expect("shift is resolved");
    assert!(matches!(
        resolved.kind,
        MethodKind::User {
            receiver: ReceiverKind::Exclusive,
            ..
        }
    ));
    assert_eq!(resolved.receiver_adjust, ReceiverAdjust::ExclusiveBorrow);
    assert_eq!(resolved.arg_tys.len(), 2);

    let sum = tail(&hir, body);
    let resolved = results.method(sum).expect("sum is resolved");
    assert!(matches!(
        resolved.kind,
        MethodKind::User {
            receiver: ReceiverKind::Shared,
            ..
        }
    ));
    assert_eq!(resolved.receiver_adjust, ReceiverAdjust::SharedBorrow);
}

#[test]
fn a_receiver_of_the_exact_reference_type_needs_no_adjustment() {
    let source = format!(
        "{POINT} func main() -> i32 {{ \
            let point = Point::new(1, 2); \
            let view: *Point = &point; \
            view.sum() \
        }}"
    );
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "main");
    let call = tail(&hir, body);
    let resolved = results.method(call).expect("sum is resolved");
    assert_eq!(resolved.receiver_adjust, ReceiverAdjust::None);
    assert!(matches!(
        resolved.kind,
        MethodKind::User {
            receiver: ReceiverKind::Shared,
            ..
        }
    ));
}

#[test]
fn associated_functions_are_called_through_the_type_namespace() {
    let source = format!("{POINT} func main() -> i32 {{ Point::new(20, 22).sum() }}");
    let (fixture, hir, results) = check_clean(&source);
    let (_, body) = function(&fixture, &hir, "main");
    let ExprKind::MethodCall { receiver, .. } = hir.expr(tail(&hir, body)).kind.clone() else {
        panic!("expected a method call");
    };
    let ExprKind::Call { callee, .. } = hir.expr(receiver).kind.clone() else {
        panic!("expected an associated call");
    };
    let definition = match hir.expr(callee).kind {
        ExprKind::Path {
            res: seal_hir::Res::Function(definition),
            ..
        } => definition,
        ref other => panic!("expected a function path, got {other:?}"),
    };
    let signature = results
        .signature(definition)
        .expect("associated function has a signature");
    assert_eq!(signature.params.len(), 2);
}

#[test]
fn a_method_uses_the_impl_namespace_instead_of_the_intrinsic_catalog() {
    check_clean(
        "struct Number { value: i32 } \
         impl Number { func abs(self: *Number) -> i32 { (*self).value } } \
         func main() -> i32 { Number { value: -3 }.abs() }",
    );
}

#[test]
fn missing_methods_on_structs_are_reported() {
    let source = format!("{POINT} func main() -> i32 {{ Point::new(1, 2).missing() }}");
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3027", "no method named `missing`");
}

#[test]
fn associated_functions_called_as_methods_are_rejected() {
    let source = format!("{POINT} func main() -> i32 {{ Point::new(1, 2).new(3, 4) }}");
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E2019", "associated function, not a method");
}

#[test]
fn user_method_arity_excludes_the_receiver() {
    let source = format!("{POINT} func main() -> i32 {{ Point::new(1, 2).shift(3) }}");
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3028", "takes 2 argument(s) but 1 were supplied");
}

#[test]
fn an_incompatible_receiver_is_reported() {
    let source = format!(
        "{POINT} func main() -> i32 {{ \
            let point = Point::new(1, 2); \
            let view: *Point = &point; \
            view.into_x() \
        }}"
    );
    let (fixture, _, _) = check_errors(&source);
    fixture.expect(
        "E3031",
        "expects `Point`, but the receiver has type `*Point`",
    );
}

#[test]
fn exclusive_methods_need_a_writable_receiver() {
    let source =
        format!("{POINT} func main() {{ let point = Point::new(1, 2); point.shift(3, 4); }}");
    let (fixture, _, _) = check_errors(&source);
    fixture.expect("E3029", "needs to modify its receiver");
}

#[test]
fn value_receivers_move_non_copy_structs() {
    let (fixture, hir, results) = check_clean(
        "struct Token { value: i32 } \
         impl Token { func consume(self: Token) -> i32 { self.value } } \
         func main() -> i32 { Token { value: 41 }.consume() + 1 }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let call = match hir.expr(tail(&hir, body)).kind.clone() {
        ExprKind::Binary { lhs, .. } => lhs,
        other => panic!("expected a binary expression, got {other:?}"),
    };
    let resolved = results.method(call).expect("consume is resolved");
    assert_eq!(resolved.receiver_adjust, ReceiverAdjust::None);
    assert!(matches!(
        resolved.kind,
        MethodKind::User {
            receiver: ReceiverKind::Value,
            ..
        }
    ));
}

fn method_expr(hir: &seal_hir::Hir, statement: seal_hir::HirStmtId) -> seal_hir::HirExprId {
    let expr = super::support::expr_statement(hir, statement);
    assert!(matches!(hir.expr(expr).kind, ExprKind::MethodCall { .. }));
    expr
}
