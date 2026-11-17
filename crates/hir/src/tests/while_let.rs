//! Lowering `while let` statements.

use crate::{PatternKind, StmtKind};

use super::support::{Fixture, function, lower_clean};

#[test]
fn a_while_let_pattern_binds_inside_the_body() {
    let (fixture, hir) = lower_clean(
        "enum Option { None, Some(i32) } \
         func next() -> Option { Option::None } \
         func main() -> i32 { \
           let mut total = 0; \
           while let Option::Some(inner) = next() { total += inner; } \
           total \
         }",
    );
    let (owner, body) = function(&fixture, &hir, "main");
    let statement = hir.block(body.root).statements[1];
    let StmtKind::WhileLet {
        pattern,
        body: loop_body,
        ..
    } = hir.stmt(statement).kind.clone()
    else {
        panic!("expected a lowered while let");
    };
    let PatternKind::Variant { fields, .. } = &hir.pattern(pattern).kind else {
        panic!("expected a variant pattern");
    };
    let (_, binding) = *fields.first().expect("the variant has one field");
    let PatternKind::Binding { def, .. } = hir.pattern(binding).kind else {
        panic!("expected a binding field pattern");
    };
    assert_eq!(hir.definitions().get(def).parent, Some(owner));
    assert_eq!(hir.block(loop_body).statements.len(), 1);
}

#[test]
fn the_while_let_binding_is_not_visible_after_the_loop() {
    let fixture = Fixture::new("func main() { while let inner = 1 { } inner }");
    let _ = fixture.lower();
    fixture.expect("E2002", "cannot find `inner`");
}
