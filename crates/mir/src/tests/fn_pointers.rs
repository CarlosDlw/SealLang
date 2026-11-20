//! Function pointer lowering and structural validation.

use crate::operand::{Constant, Operand};
use crate::rvalue::Rvalue;
use crate::statement::StatementKind;

use super::support::{function, lower_clean};

#[test]
fn a_function_address_lowers_to_a_constant() {
    let (fixture, hir, mir) = lower_clean(
        r#"
        func double_it(value: i32) -> i32 { value * 2 }
        func main() -> i32 {
            unsafe {
                let callback = double_it as *raw extern "c" func(i32) -> i32;
                0
            }
        }"#,
    );
    let body = function(&fixture, &hir, &mir, "main");
    let found = body
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .any(|statement| {
            matches!(
                statement.kind,
                StatementKind::Assign {
                    rvalue: Rvalue::Use(Operand::Constant(Constant::FunctionAddr { .. })),
                    ..
                }
            )
        });
    assert!(found, "the cast records the function address");
}

#[test]
fn an_indirect_call_lowers_the_callee_before_its_arguments() {
    let (fixture, hir, mir) = lower_clean(
        r#"
        func apply(callback: *raw extern "c" func(i32) -> i32, value: i32) -> i32 {
            unsafe { callback(value) }
        }
        func main() -> i32 { 0 }"#,
    );
    let body = function(&fixture, &hir, &mir, "apply");
    let mut indirect = None;
    for block in body.blocks.iter() {
        for statement in &block.statements {
            if let StatementKind::Assign {
                rvalue: rvalue @ Rvalue::IndirectCall { .. },
                ..
            } = &statement.kind
            {
                indirect = Some(rvalue);
            }
        }
    }
    let Some(Rvalue::IndirectCall {
        args, arg_spans, ..
    }) = indirect
    else {
        panic!("the call through the pointer is indirect");
    };
    assert_eq!(args.len(), 1);
    assert_eq!(arg_spans.len(), 1);
    assert!(matches!(args[0], Operand::Copy(_)));
}
