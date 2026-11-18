//! Lowering of custom `IntoIterator` and `Iterator` loops.
//!
//! Type checking records the protocol calls. Lowering must evaluate the source
//! once, build the condition/bind/body/exit CFG, and route `continue` back to
//! the condition rather than to an increment block.

use crate::body::Body;
use crate::id::BasicBlockId;
use crate::operand::Operand;
use crate::rvalue::{BinOp, CallTarget, Rvalue};
use crate::terminator::TerminatorKind;

use super::support::{assignment, function, lower_clean};

const CHAIN: &str = "\
struct Source { value: i32 } \
struct Cursor { value: i32 } \
impl Iterator<i32> for Cursor { \
    func has_next(self: *Cursor) -> bool { (*self).value < 3 } \
    func next(self: *mut Cursor) -> i32 { \
        let value = (*self).value; \
        (*self).value += 1; \
        value \
    } \
} \
impl IntoIterator<Cursor, i32> for Source { \
    func iter(self: Source) -> Cursor { Cursor { value: self.value } } \
}";

fn values(body: &Body) -> Vec<(&crate::place::Place, &Rvalue)> {
    body.blocks
        .iter()
        .flat_map(|block| &block.statements)
        .map(assignment)
        .collect()
}

fn call_name(
    fixture: &super::support::Fixture,
    hir: &seal_hir::Hir,
    rvalue: &Rvalue,
) -> Option<String> {
    let Rvalue::Call { target, .. } = rvalue else {
        return None;
    };
    let name = hir.definitions().name(target.instance().def);
    Some(fixture.symbol(name))
}

fn call_names(fixture: &super::support::Fixture, hir: &seal_hir::Hir, body: &Body) -> Vec<String> {
    values(body)
        .into_iter()
        .filter_map(|(_, rvalue)| call_name(fixture, hir, rvalue))
        .collect()
}

fn block_with_call(
    fixture: &super::support::Fixture,
    hir: &seal_hir::Hir,
    body: &Body,
    name: &str,
) -> BasicBlockId {
    body.blocks
        .iter_enumerated()
        .find(|(_, block)| {
            block.statements.iter().any(|statement| {
                call_name(fixture, hir, assignment(statement).1).as_deref() == Some(name)
            })
        })
        .map(|(block, _)| block)
        .unwrap_or_else(|| panic!("no block calls `{name}`"))
}

fn block_has_binary(body: &Body, block: BasicBlockId, op: BinOp) -> bool {
    body.block(block).statements.iter().any(|statement| {
        matches!(
            assignment(statement).1,
            Rvalue::Binary { op: candidate, .. } if *candidate == op
        )
    })
}

#[test]
fn a_custom_source_lowers_to_iter_has_next_and_next_once() {
    let source = format!(
        "{CHAIN} func main() -> i32 {{ \
           let source = Source {{ value: 0 }}; \
           let mut total = 0; \
           for value in source {{ total += value; }} \
           total \
         }}"
    );
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "main");
    assert_eq!(
        call_names(&fixture, &hir, body),
        ["iter", "has_next", "next"],
        "the protocol calls are emitted once in expansion order"
    );

    let iter_call = values(body)
        .into_iter()
        .find(|(_, rvalue)| call_name(&fixture, &hir, rvalue).as_deref() == Some("iter"))
        .map(|(_, rvalue)| rvalue)
        .expect("the loop calls iter");
    let Rvalue::Call { target, args, .. } = iter_call else {
        unreachable!("call_name matched a call");
    };
    assert!(matches!(target, CallTarget::Direct { .. }));
    assert_eq!(args.len(), 1, "iter only receives the source");
    assert!(
        matches!(args[0], Operand::Move(_)),
        "a non-Copy source is moved into iter"
    );
}

#[test]
fn the_source_expression_runs_once_before_iter() {
    let source = format!(
        "{CHAIN} \
         func make() -> Source {{ Source {{ value: 0 }} }} \
         func main() -> i32 {{ \
           let mut total = 0; \
           for value in make() {{ total += value; }} \
           total \
         }}"
    );
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "main");
    assert_eq!(
        call_names(&fixture, &hir, body),
        ["make", "iter", "has_next", "next"],
        "the source expression is evaluated once and before iter"
    );
}

#[test]
fn the_false_condition_branch_skips_the_body_and_next() {
    let source = format!(
        "{CHAIN} func main() -> i32 {{ \
           let source = Source {{ value: 0 }}; \
           let mut total = 0; \
           for value in source {{ total += value; }} \
           total \
         }}"
    );
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "main");
    let condition = block_with_call(&fixture, &hir, body, "has_next");
    let TerminatorKind::SwitchInt {
        targets, otherwise, ..
    } = &body.block(condition).terminator.kind
    else {
        panic!("the condition ends in a switch");
    };
    let bind = targets[0].target;
    assert_ne!(bind, *otherwise, "true goes to bind and false to exit");
    assert!(
        !body.block(*otherwise).statements.iter().any(|statement| {
            call_name(&fixture, &hir, assignment(statement).1).as_deref() == Some("next")
        }),
        "the false path never calls next"
    );
}

#[test]
fn continue_targets_the_condition_and_break_targets_the_exit() {
    let source = format!(
        "{CHAIN} func main() -> i32 {{ \
           let source = Source {{ value: 0 }}; \
           let mut total = 0; \
           for value in source {{ \
               if value == 1 {{ continue; }} \
               if value == 2 {{ break; }} \
               total += value; \
           }} \
           total \
         }}"
    );
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "main");
    let condition = block_with_call(&fixture, &hir, body, "has_next");
    let TerminatorKind::SwitchInt { otherwise, .. } = &body.block(condition).terminator.kind else {
        panic!("the condition ends in a switch");
    };
    let exit = *otherwise;

    let mut continue_seen = false;
    let mut break_seen = false;
    for (block_id, block) in body.blocks.iter_enumerated() {
        let TerminatorKind::SwitchInt { targets, .. } = &block.terminator.kind else {
            continue;
        };
        if block_has_binary(body, block_id, BinOp::Eq) {
            for target in targets {
                if let TerminatorKind::Goto(next) = &body.block(target.target).terminator.kind {
                    continue_seen |= *next == condition;
                    break_seen |= *next == exit;
                }
            }
        }
    }
    assert!(continue_seen, "continue jumps back to has_next");
    assert!(break_seen, "break jumps to the loop exit");
}

#[test]
fn a_non_copy_item_moves_into_the_pattern_binding() {
    let source = "\
        struct Item { value: i32 } \
        struct Items { value: i32 } \
        struct ItemCursor { value: i32 } \
        impl Iterator<Item> for ItemCursor { \
            func has_next(self: *ItemCursor) -> bool { (*self).value < 1 } \
            func next(self: *mut ItemCursor) -> Item { \
                (*self).value += 1; \
                Item { value: (*self).value } \
            } \
        } \
        impl IntoIterator<ItemCursor, Item> for Items { \
            func iter(self: Items) -> ItemCursor { ItemCursor { value: 0 } } \
        } \
        func main() -> i32 { \
            let items = Items { value: 0 }; \
            let mut total = 0; \
            for item in items { total += item.value; } \
            total \
        }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");
    let (next_place, _) = values(body)
        .into_iter()
        .find(|(_, rvalue)| call_name(&fixture, &hir, rvalue).as_deref() == Some("next"))
        .expect("the loop calls next");
    assert_eq!(
        fixture.ty_text(&hir, body.local(next_place.expect_local()).ty),
        "Item"
    );
    assert!(
        values(body).into_iter().any(|(_, rvalue)| {
            matches!(
                rvalue,
                Rvalue::Use(Operand::Move(place)) if place.expect_local() == next_place.expect_local()
            )
        }),
        "the binding moves the non-Copy item out of the next temporary"
    );
}

#[test]
fn nested_custom_loops_each_emit_their_own_protocol_calls() {
    let source = format!(
        "{CHAIN} func main() -> i32 {{ \
           let outer = Source {{ value: 0 }}; \
           let mut total = 0; \
           for value in outer {{ \
               let inner = Source {{ value: 1 }}; \
               for nested in inner {{ total += value + nested; }} \
           }} \
           total \
         }}"
    );
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "main");
    let names = call_names(&fixture, &hir, body);
    assert_eq!(names.iter().filter(|name| *name == "iter").count(), 2);
    assert_eq!(names.iter().filter(|name| *name == "has_next").count(), 2);
    assert_eq!(names.iter().filter(|name| *name == "next").count(), 2);
}

#[test]
fn a_direct_iterator_source_skips_iter() {
    let source = format!(
        "{CHAIN} func main() -> i32 {{ \
           let cursor = Cursor {{ value: 0 }}; \
           let mut total = 0; \
           for value in cursor {{ total += value; }} \
           total \
         }}"
    );
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "main");
    assert_eq!(
        call_names(&fixture, &hir, body),
        ["has_next", "next"],
        "the direct fallback uses the source as its own iterator"
    );
}

#[test]
fn generic_bounds_lower_to_symbolic_calls() {
    let source = format!(
        "{CHAIN} \
         func chained<S: IntoIterator<Cursor, i32>>(source: S) -> i32 {{ \
           let mut total = 0; \
           for value in source {{ total += value; }} \
           total \
         }} \
         func direct<I: Iterator<i32>>(iter: I) -> i32 {{ \
           let mut total = 0; \
           for value in iter {{ total += value; }} \
           total \
         }}"
    );
    let (fixture, hir, mir) = lower_clean(&source);

    let body = function(&fixture, &hir, &mir, "chained");
    for name in ["iter", "next"] {
        let call = values(body)
            .into_iter()
            .find(|(_, rvalue)| call_name(&fixture, &hir, rvalue).as_deref() == Some(name))
            .map(|(_, rvalue)| rvalue)
            .expect("the symbolic call is emitted");
        let Rvalue::Call { target, .. } = call else {
            unreachable!("call_name matched a call");
        };
        if name == "iter" {
            assert!(
                matches!(target, CallTarget::TraitMethod { .. }),
                "the IntoIterator bound stays symbolic"
            );
        } else {
            assert!(
                matches!(target, CallTarget::Direct { .. }),
                "the concrete Cursor Iterator is a direct call"
            );
        }
    }

    let body = function(&fixture, &hir, &mir, "direct");
    for name in ["has_next", "next"] {
        let call = values(body)
            .into_iter()
            .find(|(_, rvalue)| call_name(&fixture, &hir, rvalue).as_deref() == Some(name))
            .map(|(_, rvalue)| rvalue)
            .expect("the direct symbolic call is emitted");
        let Rvalue::Call { target, .. } = call else {
            unreachable!("call_name matched a call");
        };
        assert!(
            matches!(target, CallTarget::TraitMethod { .. }),
            "the direct Iterator bound stays symbolic"
        );
    }
}
