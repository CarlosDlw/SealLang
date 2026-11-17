//! Lowering `if let` expressions.

use crate::{ExprKind, PatternKind};

use super::support::{Fixture, function, lower_clean};

#[test]
fn an_if_let_pattern_binds_inside_the_then_branch() {
    let (fixture, hir) = lower_clean(
        "enum Option { None, Some(i32) } \
         func main(value: Option) -> i32 { \
           if let Option::Some(inner) = value { inner } else { 0 } \
         }",
    );
    let (owner, body) = function(&fixture, &hir, "main");
    let tail = hir
        .block(body.root)
        .tail
        .expect("main has a tail expression");
    let ExprKind::IfLet {
        pattern,
        then_branch,
        ..
    } = hir.expr(tail).kind.clone()
    else {
        panic!("expected a lowered if let");
    };
    assert!(hir.block(then_branch).tail.is_some());

    let PatternKind::Variant { fields, .. } = &hir.pattern(pattern).kind else {
        panic!("expected a variant pattern");
    };
    let (_, binding) = *fields.first().expect("the variant has one field");
    let PatternKind::Binding { def, .. } = hir.pattern(binding).kind else {
        panic!("expected a binding field pattern");
    };
    assert_eq!(hir.definitions().get(def).parent, Some(owner));
}

#[test]
fn the_if_let_binding_is_not_visible_after_the_expression() {
    let fixture = Fixture::new(
        "enum Option { None, Some(i32) } \
         func main(value: Option) -> i32 { \
           if let Option::Some(inner) = value { inner } else { 0 }; \
           inner \
         }",
    );
    let _ = fixture.lower();
    fixture.expect("E2002", "cannot find `inner`");
}

#[test]
fn the_if_let_binding_is_not_visible_in_the_else_branch() {
    let fixture = Fixture::new(
        "enum Option { None, Some(i32) } \
         func main(value: Option) -> i32 { \
           if let Option::Some(inner) = value { inner } else { inner } \
         }",
    );
    let _ = fixture.lower();
    fixture.expect("E2002", "cannot find `inner`");
}
