//! Tests for conditionals, loops, and scope boundaries.

use crate::{ExprKind, Res, StmtKind};

use super::support::{Fixture, function, lower_clean, path_res, tail};

#[test]
fn if_conditions_resolve_locals() {
    let (fixture, hir) = lower_clean("func main() { let flag = true; if flag { } }");
    let (_, body) = function(&fixture, &hir, "main");
    let condition = match hir.expr(tail(&hir, body)).kind.clone() {
        ExprKind::If { condition, .. } => condition,
        other => panic!("expected an if, got {other:?}"),
    };
    assert!(matches!(path_res(&hir, condition), Res::Local(_)));
}

#[test]
fn else_if_is_a_block_whose_tail_is_another_if() {
    let (fixture, hir) = lower_clean(
        "func classify(n: i32) -> i32 { if n > 0 { 1 } else if n < 0 { -1 } else { 0 } }",
    );
    let (_, body) = function(&fixture, &hir, "classify");
    let else_branch = match hir.expr(tail(&hir, body)).kind.clone() {
        ExprKind::If { else_branch, .. } => else_branch.expect("the chain has an else"),
        other => panic!("expected an if, got {other:?}"),
    };
    let nested = hir
        .block(else_branch)
        .tail
        .expect("the else-if block has a tail");
    assert!(matches!(hir.expr(nested).kind.clone(), ExprKind::If { .. }));
}

#[test]
fn if_branches_have_their_own_scope() {
    let fixture = Fixture::new("func main() { if true { let inner = 1; } inner }");
    let hir = fixture.lower();

    fixture.expect("E2002", "cannot find `inner`");
    let (_, body) = function(&fixture, &hir, "main");
    let expr = tail(&hir, body);
    assert!(matches!(hir.expr(expr).kind.clone(), ExprKind::Error));
}

#[test]
fn while_bodies_have_their_own_scope() {
    let fixture = Fixture::new("func main() { while true { let inside = 1; } inside }");
    let hir = fixture.lower();

    fixture.expect("E2002", "cannot find `inside`");
    let (_, body) = function(&fixture, &hir, "main");
    let expr = tail(&hir, body);
    assert!(matches!(hir.expr(expr).kind.clone(), ExprKind::Error));
}

#[test]
fn break_belongs_to_the_innermost_loop() {
    let (fixture, hir) =
        lower_clean("func main() { while true { while true { break; } continue; } }");
    let (_, body) = function(&fixture, &hir, "main");
    let outer = match hir.stmt(hir.block(body.root).statements[0]).kind.clone() {
        StmtKind::While { body, .. } => body,
        other => panic!("expected an outer while, got {other:?}"),
    };
    let outer_statements = &hir.block(outer).statements;
    let inner = match hir.stmt(outer_statements[0]).kind.clone() {
        StmtKind::While { body, .. } => body,
        other => panic!("expected an inner while, got {other:?}"),
    };
    assert!(matches!(
        hir.stmt(hir.block(inner).statements[0]).kind.clone(),
        StmtKind::Break
    ));
    assert!(matches!(
        hir.stmt(outer_statements[1]).kind.clone(),
        StmtKind::Continue
    ));
}
