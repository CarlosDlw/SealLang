//! Tuple lowering: structural types, literals, places, and patterns.

use seal_ty::TyKind;

use super::support::{Fixture, function, lower_clean, tail};
use crate::{ExprKind, PatternKind, Projection};

#[test]
fn tuple_types_lower_to_structural_types() {
    let (fixture, hir) = lower_clean("func main(pair: (i32, bool)) -> (i32, bool) { pair }");
    let (_, body) = function(&fixture, &hir, "main");
    let TyKind::Tuple(tuple) = hir.ty_ctxt().kind(body.ret.expect("return type")) else {
        panic!("expected a tuple type");
    };
    let elements = hir.ty_ctxt().tuple_elements(tuple);
    assert_eq!(elements.len(), 2);
    assert!(matches!(hir.ty_ctxt().kind(elements[1]), TyKind::Bool));
}

#[test]
fn tuple_literals_keep_elements_in_source_order() {
    let (fixture, hir) = lower_clean("func main() { (1, true, 3u8) }");
    let (_, body) = function(&fixture, &hir, "main");
    let expression = tail(&hir, body);
    let ExprKind::Tuple { elements } = &hir.expr(expression).kind else {
        panic!("expected a tuple literal");
    };
    assert_eq!(elements.len(), 3);
}

#[test]
fn tuple_access_lowers_to_a_positional_place() {
    let (fixture, hir) = lower_clean("func main() -> i32 { let pair = (1, 2); pair.1 }");
    let (_, body) = function(&fixture, &hir, "main");
    let expression = tail(&hir, body);
    assert!(matches!(
        hir.expr(expression).kind,
        ExprKind::TupleField { index: 1, .. }
    ));
    let place = hir.place(expression).expect("tuple access is a place");
    assert!(matches!(place.projections(), [Projection::TupleField(1)]));
}

#[test]
fn tuple_patterns_nest_in_hir() {
    let (fixture, hir) = lower_clean(
        "func main(pair: (i32, (i32, i32))) -> i32 { \
           match pair { (first, (second, third)) => first + second + third } \
         }",
    );
    let (_, body) = function(&fixture, &hir, "main");
    let expression = tail(&hir, body);
    let ExprKind::Match { arms, .. } = &hir.expr(expression).kind else {
        panic!("expected a match");
    };
    let PatternKind::Tuple(elements) = &hir.pattern(arms[0].pattern).kind else {
        panic!("expected a tuple pattern");
    };
    assert_eq!(elements.len(), 2);
    assert!(matches!(
        hir.pattern(elements[1]).kind,
        PatternKind::Tuple(_)
    ));
}

#[test]
fn a_tuple_field_of_a_temporary_is_rejected() {
    let fixture = Fixture::new("func main() -> i32 { (1, 2).0 }");
    let _ = fixture.lower();
    fixture.expect("E2012", "requires a place");
}
