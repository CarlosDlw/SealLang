//! MIR lowering of opt-in `Copy` values.
//!
//! Lowering decides between a move and a copy when it builds an operand, and
//! decides whether a scope, loop, or `drop` needs a drop site from
//! `needs_drop`. A `Copy` ADT must take the copy path and produce no drop
//! statements or drop flags.

use crate::LocalKind;
use crate::body::Body;
use crate::operand::Operand;
use crate::rvalue::Rvalue;
use crate::statement::StatementKind;

use super::support::{assignments, function, local_named, lower_clean};

const POINT: &str = "struct Point { x: i32, y: i32 } impl Copy for Point {}";

fn has_drop_statement(body: &Body) -> bool {
    body.blocks.iter().any(|block| {
        block
            .statements
            .iter()
            .any(|statement| matches!(&statement.kind, StatementKind::Drop { .. }))
    })
}

fn has_drop_remaining(body: &Body) -> bool {
    body.blocks.iter().any(|block| {
        block
            .statements
            .iter()
            .any(|statement| matches!(&statement.kind, StatementKind::DropRemaining { .. }))
    })
}

#[test]
fn assigning_a_copy_struct_uses_a_copy_operand() {
    let source = format!(
        "{POINT} func main() {{ let source = Point {{ x: 1, y: 2 }}; let copy = source; }}"
    );
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "main");
    let source = local_named(&fixture, body, "source");
    let copy = local_named(&fixture, body, "copy");
    let (_, rvalue) = assignments(body)
        .into_iter()
        .find(|(place, _)| place.expect_local() == copy)
        .expect("the binding is initialized");
    assert!(matches!(
        rvalue,
        Rvalue::Use(Operand::Copy(place)) if place.expect_local() == source
    ));
}

#[test]
fn a_copy_struct_needs_no_drop_site_or_drop_flag() {
    let source = format!(
        "{POINT} \
         func main() {{ \
           let point = Point {{ x: 1, y: 2 }}; \
           drop(point); \
         }}"
    );
    let (fixture, hir, mut mir) = lower_clean(&source);
    crate::elaborate_drops(&hir, &mut mir).expect("drop elaboration preserves its invariants");
    let body = function(&fixture, &hir, &mir, "main");
    assert!(
        !has_drop_statement(body),
        "a Copy ADT must not have a drop statement"
    );
    assert!(
        body.locals
            .iter()
            .all(|declaration| declaration.kind != LocalKind::DropFlag),
        "a Copy ADT must not need a drop flag"
    );
}

#[test]
fn a_for_loop_over_copy_items_needs_no_drop_remaining() {
    let source = format!("{POINT} func main(points: [Point; 2]) {{ for point in points {{ }} }}");
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "main");
    assert!(
        !has_drop_remaining(body),
        "a Copy array leaves no remaining elements to drop"
    );
    assert!(
        !has_drop_statement(body),
        "a Copy array needs no scope drop"
    );
}

#[test]
fn dropping_a_copy_static_emits_no_drop_statement() {
    let source = format!(
        "{POINT} static HOME: Point = Point {{ x: 1, y: 2 }}; \
         func main() {{ drop(HOME); }}"
    );
    let (fixture, _hir, mir) = lower_clean(&source);
    for body in mir.bodies() {
        assert!(
            !has_drop_statement(body),
            "no body may drop a Copy static or a Copy local"
        );
    }
    let _ = &fixture;
}

#[test]
fn a_generic_match_guard_can_use_a_copy_binding() {
    let source = "\
        struct Wrapper<T> { value: T } \
        impl<T: Copy> Copy for Wrapper<T> {} \
        func always<T>(value: T) -> bool { true } \
        func pick<T: Copy>(wrapper: Wrapper<T>) -> T { \
          match wrapper { \
            Wrapper { value } if always(value) => value, \
            Wrapper { value } => value, \
          } \
        }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "pick");
    assert!(
        body.blocks.iter().any(|block| {
            block.statements.iter().any(|statement| {
                matches!(
                    &statement.kind,
                    StatementKind::Assign {
                        rvalue: Rvalue::Call { .. },
                        ..
                    }
                )
            })
        }),
        "the guard must lower to a call after binding the Copy payload"
    );
}

#[test]
fn a_conditional_wrapper_is_copied_inside_a_generic_body() {
    let source = "\
        struct Wrapper<T> { value: T } \
        impl<T: Copy> Copy for Wrapper<T> {} \
        func duplicate<T: Copy>(value: Wrapper<T>) -> (Wrapper<T>, Wrapper<T>) { \
          (value, value) \
        }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "duplicate");
    let tuple = assignments(body)
        .into_iter()
        .find_map(|(_, rvalue)| match rvalue {
            Rvalue::Tuple { elements, .. } => Some(elements),
            _ => None,
        })
        .expect("the return value is a tuple");
    assert_eq!(tuple.len(), 2);
    assert!(
        tuple
            .iter()
            .all(|element| matches!(element, Operand::Copy(_))),
        "both tuple elements must copy the generic Copy value: {tuple:?}"
    );
}
