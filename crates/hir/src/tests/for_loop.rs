//! Lowering `for` loops.

use crate::{ForSource, PatternKind, StmtKind};

use super::support::{function, lower_clean};

#[test]
fn a_for_pattern_binds_inside_the_body_scope() {
    let (fixture, hir) = lower_clean(
        "func main(values: [i32; 2]) -> i32 { \
           let mut total = 0; \
           for value in values { total += value; } \
           total \
         }",
    );
    let (owner, body) = function(&fixture, &hir, "main");
    let statement = hir.block(body.root).statements[1];
    let StmtKind::For {
        pattern,
        source: ForSource::Expr(_),
        body: loop_body,
    } = hir.stmt(statement).kind.clone()
    else {
        panic!("expected a lowered for loop");
    };
    let PatternKind::Binding { def, .. } = hir.pattern(pattern).kind else {
        panic!("expected a binding pattern");
    };
    assert_eq!(hir.definitions().get(def).parent, Some(owner));
    assert_eq!(hir.block(loop_body).statements.len(), 1);
}

#[test]
fn the_loop_binding_is_not_visible_after_the_loop() {
    let fixture = super::support::Fixture::new(
        "func main(values: [i32; 1]) -> i32 { for value in values { } value }",
    );
    let _ = fixture.lower();
    fixture.expect("E2002", "cannot find `value`");
}
