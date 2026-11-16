//! Call lowering.

use crate::body::{Body, LocalKind};
use crate::operand::{Constant, Operand};
use crate::place::Place;
use crate::rvalue::Rvalue;
use crate::terminator::TerminatorKind;

use super::support::{assignment, function, lower_clean};

const IDENTITY: &str = "func identity(value: i32) -> i32 {\n  value\n}\n\n";

#[test]
fn a_call_in_value_position_writes_to_its_destination() {
    let source = format!("{IDENTITY}func main() -> i32 {{\n  identity(1)\n}}\n");
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "main");

    assert_eq!(
        body.locals.len(),
        1,
        "a call in return position needs no temporary"
    );
    let (place, rvalue) = assignment(&body.block(body.entry).statements[0]);
    assert_eq!(place.local, Body::return_local());

    let Rvalue::Call {
        func,
        args,
        arg_spans,
    } = rvalue
    else {
        panic!("expected a call, got {rvalue:?}");
    };
    assert_eq!(fixture.symbol(hir.definitions().name(*func)), "identity");
    assert_eq!(args.len(), 1);
    assert_eq!(arg_spans.len(), 1, "every argument records its span");
    assert!(matches!(
        args[0],
        Operand::Constant(Constant::Int { value: 1, .. })
    ));
}

#[test]
fn a_call_used_only_for_effects_gets_a_temporary() {
    let source = format!("{IDENTITY}func main() -> i32 {{\n  identity(1);\n  0\n}}\n");
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "main");

    assert_eq!(body.locals.len(), 2, "return place plus the call temporary");
    let block = body.block(body.entry);
    let (place, rvalue) = assignment(&block.statements[0]);
    assert_eq!(body.local(place.local).kind, LocalKind::Temp);
    assert!(matches!(rvalue, Rvalue::Call { .. }));

    let (place, rvalue) = assignment(&block.statements[1]);
    assert_eq!(place.local, Body::return_local());
    assert!(matches!(
        rvalue,
        Rvalue::Use(Operand::Constant(Constant::Int { value: 0, .. }))
    ));
}

#[test]
fn nested_calls_evaluate_arguments_before_the_call() {
    let source = format!("{IDENTITY}func main() -> i32 {{\n  identity(identity(2))\n}}\n");
    let (fixture, hir, mir) = lower_clean(&source);
    let body = function(&fixture, &hir, &mir, "main");
    let block = body.block(body.entry);

    assert_eq!(block.statements.len(), 2);
    let (inner_place, inner) = assignment(&block.statements[0]);
    assert!(matches!(
        inner,
        Rvalue::Call { args, .. }
            if matches!(args[0], Operand::Constant(Constant::Int { value: 2, .. }))
    ));
    assert_eq!(body.local(inner_place.local).kind, LocalKind::Temp);

    let (outer_place, outer) = assignment(&block.statements[1]);
    assert_eq!(outer_place.local, Body::return_local());
    let Rvalue::Call { args, .. } = outer else {
        panic!("expected a call, got {outer:?}");
    };
    assert_eq!(args, &vec![Operand::Copy(Place::local(inner_place.local))]);
}

#[test]
fn calls_without_results_still_lower() {
    let source = "func noop() {}\n\nfunc main() {\n  noop();\n}\n";
    let (fixture, hir, mir) = lower_clean(source);
    let noop = function(&fixture, &hir, &mir, "noop");
    assert_eq!(noop.arg_count, 0);
    assert_eq!(
        noop.block(noop.entry).terminator.kind,
        TerminatorKind::Return
    );

    let main = function(&fixture, &hir, &mir, "main");
    let block = main.block(main.entry);
    let (place, rvalue) = assignment(&block.statements[0]);
    assert_eq!(main.local(place.local).kind, LocalKind::Temp);
    let Rvalue::Call { args, .. } = rvalue else {
        panic!("expected a call, got {rvalue:?}");
    };
    assert!(args.is_empty());
}
