//! Straight-line lowering: bindings, literals, and operators.

use crate::body::Body;
use crate::operand::{Constant, Operand};
use crate::place::Place;
use crate::rvalue::{BinOp, Rvalue, UnOp};
use crate::terminator::TerminatorKind;

use super::support::{assignment, function, local_named, lower_clean, ty_text};

#[test]
fn arithmetic_tail_writes_the_return_place() {
    let (fixture, hir, mir) =
        lower_clean("func main() -> i32 {\n  let a = 1;\n  let b = 2;\n  a + b\n}\n");
    let body = function(&fixture, &hir, &mir, "main");

    assert_eq!(body.locals.len(), 3, "return place plus two bindings");
    assert_eq!(body.blocks.len(), 1);

    let block = body.block(body.entry);
    assert_eq!(block.statements.len(), 3);
    assert_eq!(block.terminator.kind, TerminatorKind::Return);

    let (place, rvalue) = assignment(&block.statements[2]);
    assert_eq!(place.local, Body::return_local());
    let Rvalue::Binary {
        op: BinOp::Add,
        lhs,
        rhs,
    } = rvalue
    else {
        panic!("expected an addition, got {rvalue:?}");
    };
    assert!(matches!(lhs, Operand::Copy(_)));
    assert!(matches!(rhs, Operand::Copy(_)));
}

#[test]
fn literals_carry_the_type_chosen_by_the_checker() {
    let (fixture, hir, mir) = lower_clean("func main() -> i64 {\n  42\n}\n");
    let body = function(&fixture, &hir, &mir, "main");
    let (place, rvalue) = assignment(&body.block(body.entry).statements[0]);

    assert_eq!(place.local, Body::return_local());
    let Rvalue::Use(Operand::Constant(Constant::Int { value, ty })) = rvalue else {
        panic!("expected an integer constant, got {rvalue:?}");
    };
    assert_eq!(*value, 42);
    assert_eq!(ty_text(&hir, *ty), "i64");
}

#[test]
fn copying_a_binding_reads_the_source_local() {
    let (fixture, hir, mir) =
        lower_clean("func main() -> i32 {\n  let a = 1;\n  let b = a;\n  b\n}\n");
    let body = function(&fixture, &hir, &mir, "main");
    let a = local_named(&fixture, body, "a");
    let b = local_named(&fixture, body, "b");

    let (place, rvalue) = assignment(&body.block(body.entry).statements[1]);
    assert_eq!(place.local, b);
    assert_eq!(rvalue, &Rvalue::Use(Operand::Copy(Place::local(a))));
}

#[test]
fn unit_functions_assign_unit_to_the_return_place() {
    let (fixture, hir, mir) = lower_clean("func main() {}\n");
    let body = function(&fixture, &hir, &mir, "main");

    assert_eq!(body.locals.len(), 1);
    let block = body.block(body.entry);
    assert_eq!(block.statements.len(), 1);
    let (place, rvalue) = assignment(&block.statements[0]);
    assert_eq!(place.local, Body::return_local());
    assert_eq!(rvalue, &Rvalue::Use(Operand::Constant(Constant::Unit)));
    assert_eq!(block.terminator.kind, TerminatorKind::Return);
}

#[test]
fn return_statements_end_their_block() {
    let (fixture, hir, mir) = lower_clean("func main() -> i32 {\n  return 7;\n}\n");
    let body = function(&fixture, &hir, &mir, "main");

    let block = body.block(body.entry);
    assert_eq!(block.terminator.kind, TerminatorKind::Return);
    let (place, rvalue) = assignment(&block.statements[0]);
    assert_eq!(place.local, Body::return_local());
    let Rvalue::Use(Operand::Constant(Constant::Int { value, .. })) = rvalue else {
        panic!("expected an integer constant, got {rvalue:?}");
    };
    assert_eq!(*value, 7);
}

#[test]
fn compound_assignment_reads_and_writes_the_same_place() {
    let (fixture, hir, mir) =
        lower_clean("func main() -> i32 {\n  let mut a = 1;\n  a += 2;\n  a\n}\n");
    let body = function(&fixture, &hir, &mir, "main");
    let a = local_named(&fixture, body, "a");

    let (place, rvalue) = assignment(&body.block(body.entry).statements[1]);
    assert_eq!(place.local, a);
    let Rvalue::Binary {
        op: BinOp::Add,
        lhs,
        rhs,
    } = rvalue
    else {
        panic!("expected an addition, got {rvalue:?}");
    };
    assert_eq!(lhs, &Operand::Copy(Place::local(a)));
    assert!(matches!(
        rhs,
        Operand::Constant(Constant::Int { value: 2, .. })
    ));
}

#[test]
fn updates_become_binary_operations_with_one() {
    let source =
        "func main() -> i32 {\n  let mut a = 10;\n  a++;\n  ++a;\n  a--;\n  --a;\n  a\n}\n";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");
    let a = local_named(&fixture, body, "a");
    let block = body.block(body.entry);

    let ops: Vec<BinOp> = block.statements[1..5]
        .iter()
        .map(|statement| {
            let (place, rvalue) = assignment(statement);
            assert_eq!(place.local, a);
            let Rvalue::Binary { op, lhs, rhs } = rvalue else {
                panic!("expected a binary operation, got {rvalue:?}");
            };
            assert_eq!(lhs, &Operand::Copy(Place::local(a)));
            assert!(matches!(
                rhs,
                Operand::Constant(Constant::Int { value: 1, .. })
            ));
            *op
        })
        .collect();

    assert_eq!(ops, vec![BinOp::Add, BinOp::Add, BinOp::Sub, BinOp::Sub]);
}

#[test]
fn unary_operators_keep_their_operation() {
    let source = "func negate() -> i32 {\n  let a = 1;\n  -a\n}\n\n\
                  func invert() -> i32 {\n  let a = 1;\n  ~a\n}\n\n\
                  func flip() -> bool {\n  let a = true;\n  !a\n}\n";
    let (fixture, hir, mir) = lower_clean(source);

    for (name, expected) in [
        ("negate", UnOp::Neg),
        ("invert", UnOp::BitNot),
        ("flip", UnOp::Not),
    ] {
        let body = function(&fixture, &hir, &mir, name);
        let block = body.block(body.entry);
        let (place, rvalue) = assignment(&block.statements[1]);
        assert_eq!(place.local, Body::return_local());
        let operand = Place::local(local_named(&fixture, body, "a"));
        assert_eq!(
            rvalue,
            &Rvalue::Unary {
                op: expected,
                operand: Operand::Copy(operand),
            },
            "wrong lowering for `{name}`"
        );
    }
}
