//! Borrow and dereference lowering in MIR.

use crate::body::{Body, LocalKind};
use crate::operand::Operand;
use crate::place::{Place, Projection};
use crate::rvalue::Rvalue;

use super::support::{assignment, function, local_named, lower_clean};

#[test]
fn borrowing_a_local_creates_a_shared_reference() {
    let (fixture, hir, mir) = lower_clean("func make() -> *i32 {\n  let x = 1;\n  &x\n}\n");
    let body = function(&fixture, &hir, &mir, "make");
    let x = local_named(&fixture, body, "x");
    let (place, rvalue) = assignment(&body.block(body.entry).statements[1]);

    assert_eq!(place.local, Body::return_local());
    assert_eq!(rvalue, &Rvalue::Ref(Place::local(x)));
}

#[test]
fn dereference_reads_through_the_reference() {
    let (fixture, hir, mir) = lower_clean("func read(p: *i32) -> i32 {\n  *p\n}\n");
    let body = function(&fixture, &hir, &mir, "read");
    let p = local_named(&fixture, body, "p");
    let (place, rvalue) = assignment(&body.block(body.entry).statements[0]);

    assert_eq!(place.local, Body::return_local());
    assert_eq!(rvalue, &Rvalue::Use(Operand::Copy(Place::local(p).deref())));
}

#[test]
fn borrowing_through_a_reference_borrows_the_projection() {
    let (fixture, hir, mir) = lower_clean("func alias(p: *i32) -> *i32 {\n  &*p\n}\n");
    let body = function(&fixture, &hir, &mir, "alias");
    let p = local_named(&fixture, body, "p");
    let (_, rvalue) = assignment(&body.block(body.entry).statements[0]);

    assert_eq!(rvalue, &Rvalue::Ref(Place::local(p).deref()));
}

#[test]
fn reading_through_a_returned_reference_uses_a_temporary() {
    let source = "func identity(p: *i32) -> *i32 {\n  p\n}\n\n\
                  func main() -> i32 {\n  let x = 1;\n  *identity(&x)\n}\n";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");
    let block = body.block(body.entry);
    let (place, rvalue) = assignment(block.statements.last().expect("the read is a statement"));

    assert_eq!(place.local, Body::return_local());
    let Rvalue::Use(Operand::Copy(read)) = rvalue else {
        panic!("expected a copy through the reference, got {rvalue:?}");
    };
    assert_eq!(read.projections(), [Projection::Deref]);
    assert_eq!(
        body.local(read.local).kind,
        LocalKind::Temp,
        "a reference value has no binding, so lowering holds it in a temporary"
    );
}

#[test]
fn dereferencing_a_borrow_reads_through_a_temporary() {
    let (fixture, hir, mir) = lower_clean("func main() -> i32 {\n  let x = 1;\n  *&x\n}\n");
    let body = function(&fixture, &hir, &mir, "main");
    let block = body.block(body.entry);
    let (reference, borrow) = assignment(&block.statements[1]);
    assert_eq!(body.local(reference.local).kind, LocalKind::Temp);
    assert!(matches!(borrow, Rvalue::Ref(_)));

    let (place, rvalue) = assignment(&block.statements[2]);
    assert_eq!(place.local, Body::return_local());
    assert_eq!(
        rvalue,
        &Rvalue::Use(Operand::Copy(reference.clone().deref()))
    );
}

#[test]
fn borrowing_through_a_reference_value_borrows_its_temporary() {
    let source = "func identity(p: *i32) -> *i32 {\n  p\n}\n\n\
                  func main() -> i32 {\n  let x = 1;\n  let r = &*identity(&x);\n  *r\n}\n";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");
    let block = body.block(body.entry);

    let (destination, borrowed) = block
        .statements
        .iter()
        .find_map(|statement| {
            let (destination, rvalue) = assignment(statement);
            match rvalue {
                Rvalue::Ref(borrowed) if borrowed.is_deref() => Some((destination, borrowed)),
                _ => None,
            }
        })
        .expect("a borrow through a reference value is lowered");
    assert_eq!(body.local(destination.local).kind, LocalKind::Binding);
    assert_eq!(body.local(borrowed.local).kind, LocalKind::Temp);
    assert_eq!(borrowed.projections(), [Projection::Deref]);
}

#[test]
fn a_borrowed_argument_is_passed_through_a_temporary() {
    let source = "func identity(p: *i32) -> *i32 {\n  p\n}\n\n\
                  func main() -> *i32 {\n  let x = 1;\n  identity(&x)\n}\n";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");
    let block = body.block(body.entry);
    let (temp, borrow) = assignment(&block.statements[1]);
    assert_eq!(body.local(temp.local).kind, crate::body::LocalKind::Temp);
    assert!(matches!(borrow, Rvalue::Ref(_)));

    let (_, call) = assignment(&block.statements[2]);
    let Rvalue::Call { args, .. } = call else {
        panic!("expected a call, got {call:?}");
    };
    assert_eq!(args, &vec![Operand::Copy(temp.clone())]);
}
