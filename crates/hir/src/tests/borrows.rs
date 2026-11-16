//! Borrow and dereference lowering.

use crate::{ExprKind, StmtKind};

use super::support::{function, lower_clean, tail};

#[test]
fn dereference_lowers_to_a_place_with_one_projection() {
    let (fixture, hir) = lower_clean("func read(p: *i32) -> i32 { *p }");
    let (_, body) = function(&fixture, &hir, "read");
    let expr = tail(&hir, body);

    assert!(matches!(hir.expr(expr).kind, ExprKind::Deref { .. }));
    let place = hir.place(expr).expect("a dereference is a place");
    assert!(matches!(place.projections(), [crate::Projection::Deref]));
    assert_eq!(place.local_def(), body.params[0].def);
}

#[test]
fn exclusive_borrows_lower_their_kind() {
    let (fixture, hir) = lower_clean("func f(p: *mut i32) -> *mut i32 { &mut *p }");
    let (_, body) = function(&fixture, &hir, "f");
    let expr = tail(&hir, body);

    let ExprKind::Borrow { kind, .. } = hir.expr(expr).kind else {
        panic!("expected a borrow, got {:?}", hir.expr(expr).kind);
    };
    assert!(kind.is_exclusive());
}

#[test]
fn nested_dereferences_count_their_projections() {
    let (fixture, hir) = lower_clean("func read(p: **i32) -> i32 { **p }");
    let (_, body) = function(&fixture, &hir, "read");
    let expr = tail(&hir, body);

    let place = hir.place(expr).expect("a dereference is a place");
    assert_eq!(place.projections().len(), 2);
    assert!(
        place
            .projections()
            .iter()
            .all(|projection| *projection == crate::Projection::Deref)
    );
}

#[test]
fn borrow_lowers_to_its_operand_and_is_not_a_place() {
    let (fixture, hir) = lower_clean("func make() -> *i32 { let x = 1; &x }");
    let (_, body) = function(&fixture, &hir, "make");
    let expr = tail(&hir, body);

    let ExprKind::Borrow { operand, .. } = hir.expr(expr).kind.clone() else {
        panic!("expected a borrow, got {:?}", hir.expr(expr).kind);
    };
    assert!(hir.place(expr).is_none());
    assert!(hir.place(operand).expect("place").is_whole());

    let statement = hir.block(body.root).statements[0];
    let StmtKind::Let { init, .. } = hir.stmt(statement).kind.clone() else {
        panic!("expected a let statement");
    };
    let init = init.expect("the binding has an initializer");
    assert!(matches!(hir.expr(init).kind, ExprKind::Lit(_)));
}
