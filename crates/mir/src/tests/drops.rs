//! Drop sites inserted by lowering.
//!
//! A drop site exists only for a value whose type needs a destructor, and today
//! that means a type parameter: `needs_drop(T)` answers `true` because
//! monomorphization decides the concrete type. The tests therefore lower
//! generic bodies and inspect where the drops land, in which order, and on which
//! path.

use crate::body::Body;
use crate::id::{BasicBlockId, LocalId};
use crate::rvalue::Rvalue;
use crate::statement::{DropKind, StatementKind};
use crate::terminator::TerminatorKind;

use super::support::{Fixture, function, local_named, lower_clean};

/// Returns the local each scope drop in `block` names, in statement order.
fn scope_drops_in(body: &Body, block: BasicBlockId) -> Vec<LocalId> {
    body.block(block)
        .statements
        .iter()
        .filter_map(|statement| match &statement.kind {
            StatementKind::Drop {
                place,
                kind: DropKind::Scope,
            } => place.local_id(),
            _ => None,
        })
        .collect()
}

/// Returns every scope drop of `body` with the block that holds it, in block
/// order.
fn scope_drops(body: &Body) -> Vec<(BasicBlockId, LocalId)> {
    body.blocks
        .iter_enumerated()
        .flat_map(|(block, _)| {
            scope_drops_in(body, block)
                .into_iter()
                .map(move |local| (block, local))
        })
        .collect()
}

/// Returns the names of `locals`, as the source spelled them.
///
/// A temporary has no name, so it is printed with its index instead.
fn names(fixture: &Fixture, body: &Body, locals: &[LocalId]) -> Vec<String> {
    locals
        .iter()
        .map(|local| match body.local(*local).name {
            Some(name) => fixture.symbol(name),
            None => format!("#{}", local.index()),
        })
        .collect()
}

/// Returns the block holding the first scope drop of `local`.
fn block_dropping(body: &Body, local: LocalId) -> BasicBlockId {
    scope_drops(body)
        .into_iter()
        .find(|(_, dropped)| *dropped == local)
        .map(|(block, _)| block)
        .unwrap_or_else(|| panic!("no block drops {local:?}"))
}

/// Returns the position of the first statement of `block` matching `wanted`.
fn position(
    body: &Body,
    block: BasicBlockId,
    wanted: impl Fn(&StatementKind) -> bool,
) -> Option<usize> {
    body.block(block)
        .statements
        .iter()
        .position(|statement| wanted(&statement.kind))
}

/// Returns the block holding the first assignment whose rvalue matches.
fn block_with_rvalue(body: &Body, wanted: impl Fn(&Rvalue) -> bool) -> BasicBlockId {
    body.blocks
        .iter_enumerated()
        .find(|(_, block)| {
            block
                .statements
                .iter()
                .any(|statement| match &statement.kind {
                    StatementKind::Assign { rvalue, .. } => wanted(rvalue),
                    _ => false,
                })
        })
        .map(|(block, _)| block)
        .unwrap_or_else(|| panic!("no block assigns the wanted rvalue"))
}

#[test]
fn a_parameter_is_dropped_before_the_implicit_return() {
    let (fixture, hir, mir) = lower_clean("func f<T>(value: T) {}\n");
    let body = function(&fixture, &hir, &mir, "f");
    assert_eq!(
        names(&fixture, body, &scope_drops_in(body, body.entry)),
        ["value"]
    );
    assert!(matches!(
        body.blocks[body.entry].terminator.kind,
        TerminatorKind::Return
    ));
}

#[test]
fn parameters_are_dropped_in_reverse_declaration_order() {
    let (fixture, hir, mir) = lower_clean("func f<T, U>(first: T, second: U) {}\n");
    let body = function(&fixture, &hir, &mir, "f");
    assert_eq!(
        names(&fixture, body, &scope_drops_in(body, body.entry)),
        ["second", "first"]
    );
}

#[test]
fn a_binding_is_dropped_before_the_parameter_it_came_from() {
    let (fixture, hir, mir) = lower_clean("func f<T>(value: T) { let copy: T = value; }\n");
    let body = function(&fixture, &hir, &mir, "f");
    assert_eq!(
        names(&fixture, body, &scope_drops_in(body, body.entry)),
        ["copy", "value"]
    );
}

#[test]
fn an_early_return_drops_the_inner_scope_first() {
    let source =
        "func f<T>(value: T) { let outer: T = value; { let inner: T = outer; return; } }\n";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "f");
    let departs = body.entry;
    assert_eq!(
        names(&fixture, body, &scope_drops_in(body, departs)),
        ["inner", "outer", "value"],
        "the return unwinds from the innermost scope outwards"
    );
    assert!(matches!(
        body.blocks[departs].terminator.kind,
        TerminatorKind::Return
    ));
}

#[test]
fn break_drops_the_loop_body_and_not_the_scope_around_it() {
    let source = "func f<T>(value: T) { let outer: T; while false { let inner: T; break; } }\n";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "f");
    let inner = local_named(&fixture, body, "inner");
    let outer = local_named(&fixture, body, "outer");
    let leaving = block_dropping(body, inner);
    assert_eq!(
        names(&fixture, body, &scope_drops_in(body, leaving)),
        ["inner"],
        "the break drops only the body it leaves"
    );
    assert!(
        !scope_drops_in(body, leaving).contains(&outer),
        "a scope outside the loop is not touched"
    );
    assert!(matches!(
        body.blocks[leaving].terminator.kind,
        TerminatorKind::Goto(_)
    ));
}

#[test]
fn continue_drops_the_loop_body_before_jumping() {
    let source = "func f<T>(value: T) { while false { let inner: T; continue; } }\n";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "f");
    let inner = local_named(&fixture, body, "inner");
    let leaving = block_dropping(body, inner);
    assert_eq!(
        names(&fixture, body, &scope_drops_in(body, leaving)),
        ["inner"]
    );
    assert!(matches!(
        body.blocks[leaving].terminator.kind,
        TerminatorKind::Goto(_)
    ));
}

#[test]
fn a_return_inside_an_if_drops_once_on_each_path() {
    let source = "func f<T>(value: T) { if true { return; } }\n";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "f");
    let value = local_named(&fixture, body, "value");
    let drops = scope_drops(body);
    assert_eq!(
        drops.len(),
        2,
        "one site where the branch returns, one on the path that joins"
    );
    let blocks: Vec<BasicBlockId> = drops.iter().map(|(block, _)| *block).collect();
    assert_ne!(blocks[0], blocks[1], "the two sites are on different paths");
    for (block, dropped) in &drops {
        assert_eq!(*dropped, value);
        assert_eq!(
            scope_drops_in(body, *block),
            vec![value],
            "a single path drops the parameter once"
        );
    }
}

#[test]
fn a_discarded_call_result_is_dropped_at_the_end_of_its_statement() {
    let source = "func make<T>(value: T) -> T { value } \
                  func f<T>(value: T) { make(value); }\n";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "f");
    let block = body.entry;
    let call = position(body, block, |kind| {
        matches!(
            kind,
            StatementKind::Assign {
                rvalue: Rvalue::Call { .. },
                ..
            }
        )
    })
    .expect("the body calls make");
    let dropped = position(body, block, |kind| {
        matches!(
            kind,
            StatementKind::Drop {
                kind: DropKind::Scope,
                ..
            }
        )
    })
    .expect("the result temporary is dropped");
    assert!(
        dropped > call,
        "the temporary outlives the call that produced it"
    );
    let temporary = scope_drops_in(body, block)[0];
    assert_eq!(
        body.local(temporary).kind,
        crate::LocalKind::Temp,
        "the statement drops its own temporary, before the parameter"
    );
}

#[test]
fn a_borrowed_temporary_outlives_the_statement_that_built_it() {
    // The array literal has no place of its own, so lowering materializes it in
    // a temporary; the slice borrows that storage and outlives the statement.
    let source = "func first<T>(values: *[T]) -> i32 { 0 } \
                  func f<T>(a: T, b: T) -> i32 { \
                    let sliced = &[a, b][..]; \
                    first(sliced) \
                  }\n";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "f");
    let block = body.entry;
    let read = position(body, block, |kind| {
        matches!(
            kind,
            StatementKind::Assign {
                rvalue: Rvalue::Call { .. },
                ..
            }
        )
    })
    .expect("the later statement reads the borrow");
    let temporary = scope_drops_in(body, block)
        .into_iter()
        .find(|local| body.local(*local).kind == crate::LocalKind::Temp)
        .expect("the borrowed temporary is dropped with the block");
    let dropped = position(body, block, |kind| {
        matches!(kind, StatementKind::Drop { place, .. } if place.local_id() == Some(temporary))
    })
    .expect("the temporary has a drop site");
    assert!(
        dropped > read,
        "the storage stays alive while the borrow it fed is used"
    );
}

#[test]
fn a_value_without_a_destructor_gets_no_drop_site() {
    let (fixture, hir, mir) = lower_clean("func f(value: i32) {}\n");
    let body = function(&fixture, &hir, &mir, "f");
    assert!(scope_drops(body).is_empty());
}

#[test]
fn a_const_of_a_trivial_type_gets_no_new_statement() {
    let (_fixture, hir, mir) = lower_clean("const VALUE: i32 = 7;\n");
    let value = hir
        .definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == seal_def::DefKind::Const)
        .map(|(def, _)| def)
        .expect("VALUE is a const");
    let body = mir.body_of(value).expect("VALUE has a body");
    assert_eq!(
        body.block(body.entry).statements.len(),
        1,
        "only the initializer assignment is emitted"
    );
    assert!(scope_drops(body).is_empty());
}

#[test]
fn a_by_value_loop_advances_the_index_before_the_body() {
    let source = "func take<T>(value: T) {} \
                  func f<T>(items: [T; 2]) { for item in items { take(item); } }\n";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "f");
    let block = block_with_rvalue(body, |rvalue| matches!(rvalue, Rvalue::Call { .. }));
    let first = body.block(block).statements.first().expect("the body runs");
    assert!(
        matches!(
            &first.kind,
            StatementKind::Assign {
                rvalue: Rvalue::Binary { .. },
                ..
            }
        ),
        "the index advances before the body statements, got {:?}",
        first.kind
    );
}

#[test]
fn a_by_value_loop_drops_the_elements_left_behind() {
    let source = "func take<T>(value: T) {} \
                  func f<T>(items: [T; 2]) { for item in items { break; } }\n";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "f");
    let remaining = body
        .blocks
        .iter_enumerated()
        .find_map(|(block, data)| {
            data.statements
                .iter()
                .find_map(|statement| match &statement.kind {
                    StatementKind::DropRemaining { place, from } => Some((block, place, from)),
                    _ => None,
                })
        })
        .expect("the loop exit drops the remaining elements");
    assert!(
        matches!(remaining.2, crate::Operand::Copy(_)),
        "the range starts at the index the loop kept"
    );
    assert!(matches!(
        body.blocks[remaining.0].terminator.kind,
        TerminatorKind::Goto(_) | TerminatorKind::Return
    ));
}

#[test]
fn a_by_value_loop_never_drops_the_array_itself() {
    let source = "func take<T>(value: T) {} \
                  func f<T>(items: [T; 2]) { for item in items { take(item); } }\n";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "f");
    for (_, dropped) in scope_drops(body) {
        let declaration = body.local(dropped);
        assert!(
            declaration.kind != crate::LocalKind::Temp
                || !matches!(
                    hir.ty_ctxt().kind(declaration.ty),
                    seal_ty::TyKind::Array { .. }
                ),
            "the loop consumed its array, so a scope drop would drop taken elements"
        );
    }
}

#[test]
fn a_reference_loop_does_not_drop_the_elements() {
    let source = "func f<T>(items: *[T; 2]) { for item in items { } }\n";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "f");
    assert!(
        !body.blocks.iter().any(|block| block
            .statements
            .iter()
            .any(|statement| matches!(statement.kind, StatementKind::DropRemaining { .. }))),
        "a borrowing loop leaves the elements to their owner"
    );
}
