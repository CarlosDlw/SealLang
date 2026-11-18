//! Control-flow lowering: branches, loops, and short-circuit operators.

use crate::body::Body;
use crate::operand::{Constant, Operand};
use crate::place::Place;
use crate::rvalue::Rvalue;
use crate::terminator::TerminatorKind;

use super::support::{assignment, function, local_named, lower_clean};

#[test]
fn if_expressions_join_both_branches() {
    let source = "func main() -> i32 {\n  let a = 1;\n  if a > 0 { 1 } else { 2 }\n}\n";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");

    assert_eq!(body.blocks.len(), 4, "entry, then, else, and join");
    let entry = body.block(body.entry);
    let TerminatorKind::SwitchInt {
        targets, otherwise, ..
    } = &entry.terminator.kind
    else {
        panic!("expected a switch, got {:?}", entry.terminator.kind);
    };
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].value, 1);

    let then_block = targets[0].target;
    let else_block = *otherwise;
    let join = match &body.block(then_block).terminator.kind {
        TerminatorKind::Goto(join) => *join,
        other => panic!("expected a jump, got {other:?}"),
    };
    assert_eq!(
        body.block(else_block).terminator.kind,
        TerminatorKind::Goto(join)
    );
    assert_eq!(body.block(join).terminator.kind, TerminatorKind::Return);

    for block in [then_block, else_block] {
        let (place, _) = assignment(&body.block(block).statements[0]);
        assert_eq!(place.expect_local(), Body::return_local());
    }
}

#[test]
fn if_without_else_produces_an_empty_else_block() {
    let source = "func main() {\n  if true {\n  }\n  return;\n}\n";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");

    assert_eq!(body.blocks.len(), 4);
    let TerminatorKind::SwitchInt {
        targets, otherwise, ..
    } = &body.block(body.entry).terminator.kind
    else {
        panic!("expected a switch");
    };
    let then_block = targets[0].target;
    let else_block = *otherwise;
    let join = match &body.block(then_block).terminator.kind {
        TerminatorKind::Goto(join) => *join,
        other => panic!("expected a jump, got {other:?}"),
    };

    assert!(body.block(else_block).statements.is_empty());
    assert_eq!(
        body.block(else_block).terminator.kind,
        TerminatorKind::Goto(join)
    );

    let block = body.block(join);
    assert_eq!(block.terminator.kind, TerminatorKind::Return);
    assert_eq!(block.statements.len(), 1);
    assert_eq!(
        assignment(&block.statements[0]).1,
        &Rvalue::Use(Operand::Constant(Constant::Unit))
    );
}

#[test]
fn while_loops_lower_to_condition_body_and_exit_blocks() {
    let source =
        "func main() -> i32 {\n  let mut i = 0;\n  while i < 3 {\n    i += 1;\n  }\n  i\n}\n";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");

    assert_eq!(body.blocks.len(), 4, "entry, condition, body, and exit");
    let condition = match &body.block(body.entry).terminator.kind {
        TerminatorKind::Goto(condition) => *condition,
        other => panic!("expected a jump into the condition, got {other:?}"),
    };
    let TerminatorKind::SwitchInt {
        targets, otherwise, ..
    } = &body.block(condition).terminator.kind
    else {
        panic!("expected a condition switch");
    };
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].value, 1);

    let loop_body = targets[0].target;
    let exit = *otherwise;
    assert_eq!(
        body.block(loop_body).terminator.kind,
        TerminatorKind::Goto(condition)
    );

    let exit_block = body.block(exit);
    assert_eq!(exit_block.terminator.kind, TerminatorKind::Return);
    let (place, _) = assignment(exit_block.statements.last().expect("exit assigns the tail"));
    assert_eq!(place.expect_local(), Body::return_local());
}

#[test]
fn break_and_continue_target_the_enclosing_loop() {
    let source = "func main() -> i32 {\n  let mut i = 0;\n  while true {\n    i += 1;\n    \
                  if i == 2 {\n      continue;\n    }\n    if i == 3 {\n      break;\n    }\n  }\n  i\n}\n";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");

    let condition = match &body.block(body.entry).terminator.kind {
        TerminatorKind::Goto(condition) => *condition,
        other => panic!("expected a jump, got {other:?}"),
    };
    let TerminatorKind::SwitchInt {
        otherwise: exit, ..
    } = &body.block(condition).terminator.kind
    else {
        panic!("expected a condition switch");
    };

    let jumps: Vec<_> = body
        .blocks
        .iter()
        .filter_map(|block| match &block.terminator.kind {
            TerminatorKind::Goto(target) => Some(*target),
            _ => None,
        })
        .collect();
    assert!(
        jumps.contains(&condition),
        "`continue` and the loop back edge must jump to the condition"
    );
    assert!(jumps.contains(exit), "`break` must jump to the loop exit");
}

#[test]
fn logical_and_evaluates_the_right_operand_only_on_true() {
    let source = "func main() -> bool {\n  let a = true;\n  let b = false;\n  a && b\n}\n";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");
    let b = local_named(&fixture, body, "b");

    assert_eq!(
        body.blocks.len(),
        4,
        "entry, right operand, short-circuit, and join"
    );
    let entry = body.block(body.entry);
    let TerminatorKind::SwitchInt {
        targets, otherwise, ..
    } = &entry.terminator.kind
    else {
        panic!("expected a switch, got {:?}", entry.terminator.kind);
    };
    let right = targets[0].target;
    let short = *otherwise;

    assert_eq!(
        assignment(&body.block(short).statements[0]).1,
        &Rvalue::Use(Operand::Constant(Constant::Bool(false)))
    );
    assert_eq!(
        assignment(&body.block(right).statements[0]).1,
        &Rvalue::Use(Operand::Copy(Place::local(b)))
    );

    let join = match &body.block(short).terminator.kind {
        TerminatorKind::Goto(join) => *join,
        other => panic!("expected a jump, got {other:?}"),
    };
    assert_eq!(
        body.block(right).terminator.kind,
        TerminatorKind::Goto(join)
    );
    assert_eq!(body.block(join).terminator.kind, TerminatorKind::Return);
}

#[test]
fn logical_or_short_circuits_with_true() {
    let source = "func main() -> bool {\n  let a = true;\n  let b = false;\n  a || b\n}\n";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");

    let entry = body.block(body.entry);
    let TerminatorKind::SwitchInt {
        targets, otherwise, ..
    } = &entry.terminator.kind
    else {
        panic!("expected a switch, got {:?}", entry.terminator.kind);
    };
    let short = targets[0].target;
    let right = *otherwise;

    assert_eq!(
        assignment(&body.block(short).statements[0]).1,
        &Rvalue::Use(Operand::Constant(Constant::Bool(true)))
    );
    assert!(matches!(
        assignment(&body.block(right).statements[0]).1,
        Rvalue::Use(Operand::Copy(_))
    ));
}

#[test]
fn statements_after_return_stay_in_unreachable_blocks() {
    let source = "func main() -> i32 {\n  return 0;\n  let x = 1;\n  x\n}\n";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");

    assert_eq!(body.blocks.len(), 2);
    assert_eq!(
        body.block(body.entry).terminator.kind,
        TerminatorKind::Return
    );

    let unreachable = body
        .blocks
        .iter_enumerated()
        .find(|(block, _)| *block != body.entry)
        .map(|(_, block)| block)
        .expect("lowering keeps the block after divergence");
    assert_eq!(unreachable.terminator.kind, TerminatorKind::Return);
    assert_eq!(unreachable.statements.len(), 2);
}
