//! Lowering of the built-in `drop(value)`.

use super::support::{Fixture, expr_statement, function, lower_clean, tail};
use crate::ExprKind;

#[test]
fn a_drop_call_lowers_to_a_dedicated_node() {
    let source = "\
struct Point { x: i32 } \
func release(point: Point) { drop(point); }";
    let (fixture, hir) = lower_clean(source);
    let (_, body) = function(&fixture, &hir, "release");
    let expr = expr_statement(&hir, hir.block(body.root).statements[0]);
    assert!(matches!(hir.expr(expr).kind, ExprKind::Drop { .. }));
}

#[test]
fn drop_requires_exactly_one_argument() {
    let fixture = Fixture::new("func main() { drop(); }");
    let _ = fixture.lower();
    fixture.expect("E3008", "takes exactly one argument");
}

#[test]
fn drop_rejects_two_arguments() {
    let fixture = Fixture::new("func main() { drop(1, 2); }");
    let _ = fixture.lower();
    fixture.expect("E3008", "takes exactly one argument");
}

#[test]
fn a_user_function_named_drop_shadows_the_builtin() {
    let source = "\
func drop(value: i32) -> i32 { value } \
func main() -> i32 { drop(1) }";
    let (fixture, hir) = lower_clean(source);
    let (_, body) = function(&fixture, &hir, "main");
    let call = tail(&hir, body);
    assert!(matches!(hir.expr(call).kind, ExprKind::Call { .. }));
}

#[test]
fn a_binding_named_drop_shadows_the_builtin() {
    let fixture = Fixture::new("func main() { let drop = 5; drop(2); }");
    let hir = fixture.lower();
    let (_, body) = function(&fixture, &hir, "main");
    let call = expr_statement(&hir, hir.block(body.root).statements[1]);
    assert!(
        !matches!(hir.expr(call).kind, ExprKind::Drop { .. }),
        "a binding named `drop` must shadow the built-in"
    );
}
