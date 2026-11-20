//! Seal function pointer lowering and structural validation.

use seal_ty::{IntTy, TyKind};

use crate::operand::{Constant, Operand};
use crate::rvalue::{CallTarget, Rvalue};
use crate::statement::StatementKind;
use crate::validate::ValidationError;

use super::support::{function, lower_clean};

#[test]
fn a_function_value_lowers_to_a_function_address() {
    let (fixture, hir, mir) = lower_clean(
        "func identity(value: i32) -> i32 { value }\n\
         func main() -> i32 {\n  let f: func(i32) -> i32 = identity;\n  f(1)\n}",
    );
    let body = function(&fixture, &hir, &mir, "main");
    let found = body
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .any(|statement| match &statement.kind {
            StatementKind::Assign {
                rvalue: Rvalue::Use(Operand::Constant(Constant::FunctionAddr { ty, .. })),
                ..
            } => hir.ty_ctxt().fn_ptr_parts(*ty).is_some(),
            _ => false,
        });
    assert!(
        found,
        "the function item becomes a function address constant"
    );
}

#[test]
fn a_call_through_a_function_pointer_names_its_pointer_type() {
    let (fixture, hir, mir) = lower_clean(
        "func apply(f: func(i32) -> i32, value: i32) -> i32 { f(value) }\n\
         func main() -> i32 { 0 }",
    );
    let body = function(&fixture, &hir, &mir, "apply");
    let mut call = None;
    for block in body.blocks.iter() {
        for statement in &block.statements {
            if let StatementKind::Assign {
                rvalue:
                    rvalue @ Rvalue::Call {
                        target: CallTarget::FnPtr { .. },
                        ..
                    },
                ..
            } = &statement.kind
            {
                call = Some(rvalue);
            }
        }
    }
    let Some(Rvalue::Call {
        target: CallTarget::FnPtr { ty, receiver },
        args,
        arg_spans,
    }) = call
    else {
        panic!("the call through the parameter is a function pointer call");
    };
    assert_eq!(*receiver, None);
    assert!(matches!(hir.ty_ctxt().kind(*ty), TyKind::FnPtr(_)));
    let definition = hir
        .ty_ctxt()
        .fn_ptr_parts(*ty)
        .expect("the target has a signature");
    assert_eq!(definition.params.len(), 1);
    assert_eq!(args.len(), 2, "the pointer is the first argument");
    assert_eq!(arg_spans.len(), args.len());
    let place = args[0].place().expect("the pointer is read from a place");
    let local = place.local_id().expect("the pointer lives in a local");
    assert!(matches!(
        hir.ty_ctxt().kind(body.local(local).ty),
        TyKind::FnPtr(_)
    ));
}

#[test]
fn a_function_pointer_call_must_pass_the_pointer() {
    let (fixture, hir, mir) = lower_clean(
        "func identity(value: i32) -> i32 { value }\n\
         func main() -> i32 {\n  let f = identity;\n  f(1)\n}",
    );
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    let statement = body.blocks[body.entry]
        .statements
        .iter_mut()
        .find(|statement| {
            matches!(
                statement.kind,
                StatementKind::Assign {
                    rvalue: Rvalue::Call {
                        target: CallTarget::FnPtr { .. },
                        ..
                    },
                    ..
                }
            )
        })
        .expect("main calls through the function pointer");
    let StatementKind::Assign {
        rvalue: Rvalue::Call { args, .. },
        ..
    } = &mut statement.kind
    else {
        unreachable!("the search selected a call");
    };
    args[0] = Operand::Constant(Constant::Int {
        value: 0,
        ty: hir.ty_ctxt().int_ty(IntTy::I32),
    });
    assert_eq!(
        body.validate_with(&hir),
        Err(ValidationError::FnPtrCalleeTypeMismatch { block: body.entry })
    );
}

#[test]
fn a_function_pointer_call_checks_its_arity() {
    let (fixture, hir, mir) = lower_clean(
        "func identity(value: i32) -> i32 { value }\n\
         func main() -> i32 {\n  let f = identity;\n  f(1)\n}",
    );
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    let span = body.blocks[body.entry].statements[0].span;
    let statement = body.blocks[body.entry]
        .statements
        .iter_mut()
        .find(|statement| {
            matches!(
                statement.kind,
                StatementKind::Assign {
                    rvalue: Rvalue::Call {
                        target: CallTarget::FnPtr { .. },
                        ..
                    },
                    ..
                }
            )
        })
        .expect("main calls through the function pointer");
    let StatementKind::Assign {
        rvalue: Rvalue::Call {
            args, arg_spans, ..
        },
        ..
    } = &mut statement.kind
    else {
        unreachable!("the search selected a call");
    };
    let repeated = args[1].clone();
    args.push(repeated);
    arg_spans.push(span);
    assert_eq!(
        body.validate_with(&hir),
        Err(ValidationError::CallArityMismatch {
            args: 2,
            parameters: 1
        })
    );
}

#[test]
fn a_function_pointer_call_checks_argument_types() {
    let (fixture, hir, mir) = lower_clean(
        "func apply(f: func(bool) -> bool, value: bool) -> bool { f(value) }\n\
         func main() -> bool { true }",
    );
    let mut body = function(&fixture, &hir, &mir, "apply").clone();
    let statement = body.blocks[body.entry]
        .statements
        .iter_mut()
        .find(|statement| {
            matches!(
                statement.kind,
                StatementKind::Assign {
                    rvalue: Rvalue::Call {
                        target: CallTarget::FnPtr { .. },
                        ..
                    },
                    ..
                }
            )
        })
        .expect("apply calls through the function pointer");
    let StatementKind::Assign {
        rvalue: Rvalue::Call { args, .. },
        ..
    } = &mut statement.kind
    else {
        unreachable!("the search selected a call");
    };
    args[1] = Operand::Constant(Constant::Int {
        value: 1,
        ty: hir.ty_ctxt().int_ty(IntTy::I32),
    });
    assert_eq!(
        body.validate_with(&hir),
        Err(ValidationError::RvalueOperandTypeMismatch { block: body.entry })
    );
}
