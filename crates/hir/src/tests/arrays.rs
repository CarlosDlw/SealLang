//! Array lowering and places.

use crate::Projection;
use crate::{ExprKind, Hir};

use super::support::{Fixture, assert_invariants, function, lower_clean, tail};

#[test]
fn array_literals_and_repeats_lower_in_source_order() {
    let (fixture, hir) =
        lower_clean("func main() -> i32 { let a = [1, 2, 3]; let b = [4; 2]; a[1] + b[0] }");
    let (_, body) = function(&fixture, &hir, "main");
    let first = match &hir.stmt(hir.block(body.root).statements[0]).kind {
        crate::StmtKind::Let { init, .. } => init.expect("initializer"),
        other => panic!("expected a let, got {other:?}"),
    };
    let ExprKind::Array { elements } = &hir.expr(first).kind else {
        panic!("expected an array literal");
    };
    assert_eq!(elements.len(), 3);
    let second = match &hir.stmt(hir.block(body.root).statements[1]).kind {
        crate::StmtKind::Let { init, .. } => init.expect("initializer"),
        other => panic!("expected a let, got {other:?}"),
    };
    let ExprKind::ArrayRepeat { value, length } = hir.expr(second).kind else {
        panic!("expected a repeated literal");
    };
    assert_eq!(length, 2);
    assert!(matches!(hir.expr(value).kind, ExprKind::Lit(_)));
    assert_invariants(&fixture, &hir);
}

#[test]
fn indexing_lowers_to_a_place_with_the_index_expression() {
    let (fixture, hir) = lower_clean("func f(a: [i32; 3], i: usize) -> i32 { a[i] }");
    let (_, body) = function(&fixture, &hir, "f");
    let expression = tail(&hir, body);
    let ExprKind::Index { base, index } = hir.expr(expression).kind else {
        panic!("expected an index expression");
    };
    let place = hir.place(expression).expect("an index is a place");
    assert_eq!(
        place.local_def(),
        hir.place(base).expect("base place").local_def()
    );
    assert_eq!(place.projections(), &[Projection::Index(index)]);
    assert_invariants(&fixture, &hir);
}

#[test]
fn indexing_a_reference_lowers_deref_before_index() {
    let (fixture, hir) = lower_clean("func f(p: *mut [i32; 2], i: usize) -> i32 { (*p)[i] }");
    let (_, body) = function(&fixture, &hir, "f");
    let place = hir.place(tail(&hir, body)).expect("indexed reference");
    assert_eq!(place.projections().len(), 2);
    assert_eq!(place.projections()[0], Projection::Deref);
    assert!(matches!(place.projections()[1], Projection::Index(_)));
    assert_invariants(&fixture, &hir);
}

#[test]
fn indexing_a_temporary_is_rejected_but_keeps_the_tree_reachable() {
    let fixture = Fixture::new("func main() -> i32 { [1, 2][0] }");
    let hir = fixture.lower();
    fixture.expect("E2013", "indexing requires a place");
    assert_invariants(&fixture, &hir);
    let (_, body) = function(&fixture, &hir, "main");
    assert!(matches!(
        hir.expr(tail(&hir, body)).kind,
        ExprKind::Index { .. }
    ));
}

#[test]
fn array_type_annotations_lower_to_one_interned_id() {
    let (fixture, hir) = lower_clean("func f(a: [i32; 4]) -> [i32; 4] { a }");
    let (owner, body) = function(&fixture, &hir, "f");
    let signature = hir.definitions().get(owner);
    let _ = signature;
    let param = body.params[0].ty;
    assert_eq!(body.ret, Some(param));
    let Hir { .. } = hir;
    assert_invariants(&fixture, &hir);
}
