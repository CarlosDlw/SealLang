//! Structural validation of MIR bodies.

use seal_ty::{InferTyId, Instance, UintTy};

use crate::body::{Body, LocalKind};
use crate::id::{BasicBlockId, LocalId};
use crate::operand::{Constant, Operand};
use crate::place::Place;
use crate::rvalue::{AggregateKind, CallTarget, Rvalue, StrViewKind};
use crate::statement::StatementKind;
use crate::terminator::TerminatorKind;
use crate::validate::ValidationError;

use super::support::{function, lower_clean};

fn str_view_block(body: &Body) -> BasicBlockId {
    body.blocks
        .iter_enumerated()
        .find(|(_, block)| {
            block.statements.iter().any(|statement| {
                matches!(
                    statement.kind,
                    StatementKind::Assign {
                        rvalue: Rvalue::StrView { .. },
                        ..
                    }
                )
            })
        })
        .map(|(block, _)| block)
        .expect("the body builds a string view")
}

fn str_view_mut(body: &mut Body) -> &mut Rvalue {
    body.blocks
        .iter_mut()
        .flat_map(|block| block.statements.iter_mut())
        .find_map(|statement| match &mut statement.kind {
            StatementKind::Assign {
                rvalue: rvalue @ Rvalue::StrView { .. },
                ..
            } => Some(rvalue),
            _ => None,
        })
        .expect("the body builds a string view")
}

fn main_body(source: &str) -> Body {
    let (fixture, hir, mir) = lower_clean(source);
    function(&fixture, &hir, &mir, "main").clone()
}

#[test]
fn calls_need_one_span_per_argument() {
    let mut body = main_body(
        "func identity(value: i32) -> i32 {\n  value\n}\n\n\
         func main() -> i32 {\n  identity(1)\n}\n",
    );
    let statement = body.blocks[body.entry]
        .statements
        .first_mut()
        .expect("main calls identity");
    let StatementKind::Assign {
        rvalue: Rvalue::Call { arg_spans, .. },
        ..
    } = &mut statement.kind
    else {
        panic!("expected a call, got {:?}", statement.kind);
    };
    arg_spans.clear();
    assert_eq!(
        body.validate(),
        Err(ValidationError::CallSpansMismatch { args: 1, spans: 0 })
    );
}

#[test]
fn only_function_bodies_may_be_const_functions() {
    let (_fixture, hir, mir) = lower_clean("const VALUE: i32 = 1;");
    let value = hir
        .definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == seal_def::DefKind::Const)
        .map(|(def, _)| def)
        .expect("VALUE is a const");
    let mut body = mir.body_of(value).expect("VALUE has a body").clone();
    body.is_const = true;
    assert_eq!(body.validate(), Err(ValidationError::ConstFlagOnValueBody));
}

#[test]
fn bodies_without_blocks_are_rejected() {
    let mut body = main_body("func main() {}\n");
    body.blocks.clear();
    assert_eq!(body.validate(), Err(ValidationError::NoBlocks));
}

#[test]
fn entries_must_name_a_block() {
    let mut body = main_body("func main() {}\n");
    body.entry = BasicBlockId::from_u32(42).expect("42 is representable");
    assert!(matches!(
        body.validate(),
        Err(ValidationError::EntryOutOfRange { .. })
    ));
}

#[test]
fn bodies_need_a_return_local() {
    let mut body = main_body("func main() {}\n");
    body.locals.clear();
    assert_eq!(body.validate(), Err(ValidationError::MissingReturnLocal));
}

#[test]
fn local_zero_must_be_the_return_place() {
    let mut body = main_body("func main() {}\n");
    body.locals[Body::return_local()].kind = LocalKind::Binding;
    assert_eq!(
        body.validate(),
        Err(ValidationError::ReturnLocalHasWrongKind {
            kind: LocalKind::Binding
        })
    );
}

#[test]
fn parameters_must_be_argument_locals() {
    let mut body = main_body("func main(value: i32) -> i32 {\n  value\n}\n");
    body.locals[LocalId::from_u32(1).expect("one is representable")].kind = LocalKind::Temp;
    assert!(matches!(
        body.validate(),
        Err(ValidationError::ArgLocalHasWrongKind { .. })
    ));
}

#[test]
fn places_must_name_an_existing_local() {
    let mut body = main_body("func main() -> i32 {\n  0\n}\n");
    let statement = body
        .blocks
        .iter_mut()
        .flat_map(|block| block.statements.iter_mut())
        .next()
        .expect("the body has one assignment");
    let StatementKind::Assign { place, .. } = &mut statement.kind else {
        panic!("expected an assignment, found {:?}", statement.kind);
    };
    *place = Place::local(LocalId::from_u32(99).expect("99 is representable"));

    assert!(matches!(
        body.validate(),
        Err(ValidationError::PlaceOutOfRange { .. })
    ));
}

#[test]
fn edges_must_name_an_existing_block() {
    let mut body = main_body("func main() -> i32 {\n  return 0;\n}\n");
    let entry = body.entry;
    body.blocks[entry].terminator.kind =
        TerminatorKind::Goto(BasicBlockId::from_u32(99).expect("99 is representable"));

    assert!(matches!(
        body.validate(),
        Err(ValidationError::BlockTargetOutOfRange { .. })
    ));
}

#[test]
fn switch_targets_are_checked() {
    let mut body = main_body("func main() {\n  if true {\n  }\n}\n");
    let entry = body.entry;
    let TerminatorKind::SwitchInt { targets, .. } = &mut body.blocks[entry].terminator.kind else {
        panic!("the entry block ends in a switch");
    };
    targets[0].target = BasicBlockId::from_u32(99).expect("99 is representable");

    assert!(matches!(
        body.validate(),
        Err(ValidationError::BlockTargetOutOfRange { .. })
    ));
}

#[test]
fn a_leftover_inference_variable_is_rejected() {
    let (fixture, hir, mir) = lower_clean("func main() -> i32 { 0 }");
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    let infer = hir
        .ty_ctxt()
        .infer_ty(InferTyId::from_u32(0).expect("zero is representable"));
    body.locals[Body::return_local()].ty = infer;
    assert_eq!(
        body.validate_with(&hir),
        Err(ValidationError::InferredType {
            local: Body::return_local()
        })
    );
}

#[test]
fn a_string_view_requires_a_string_pointer() {
    let source = "func main(text: *str, index: usize) -> *str { text.slice(index, index) }";
    let (fixture, hir, mir) = lower_clean(source);
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    let index = body
        .locals
        .iter_enumerated()
        .find(|(_, local)| hir.ty_ctxt().display(local.ty).to_string() == "usize")
        .map(|(local, _)| local)
        .expect("the index parameter has type usize");
    let block = str_view_block(&body);
    let Rvalue::StrView { base, .. } = str_view_mut(&mut body) else {
        unreachable!();
    };
    *base = Place::local(index);
    assert_eq!(
        body.validate_with(&hir),
        Err(ValidationError::StrViewBaseTypeMismatch {
            block,
            kind: StrViewKind::Substring,
        })
    );
}

#[test]
fn a_string_view_must_produce_its_declared_pointer() {
    let source = "func main(text: *str) -> *str { text.slice(0, 1) }";
    let (fixture, hir, mir) = lower_clean(source);
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    let block = str_view_block(&body);
    let Rvalue::StrView { ty, .. } = str_view_mut(&mut body) else {
        unreachable!();
    };
    *ty = hir.ty_ctxt().uint_ty(UintTy::Usize);
    assert_eq!(
        body.validate_with(&hir),
        Err(ValidationError::StrViewTargetTypeMismatch {
            block,
            kind: StrViewKind::Substring,
        })
    );
}

#[test]
fn a_string_view_cannot_record_bounds_it_does_not_use() {
    let source = "func main(text: *str) -> *[u8] { text.as_bytes() }";
    let (fixture, hir, mir) = lower_clean(source);
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    let block = str_view_block(&body);
    let Rvalue::StrView { start, .. } = str_view_mut(&mut body) else {
        unreachable!();
    };
    *start = Some(Operand::Constant(Constant::Int {
        value: 0,
        ty: hir.ty_ctxt().uint_ty(UintTy::Usize),
    }));
    assert_eq!(
        body.validate(),
        Err(ValidationError::StrViewRangeMismatch {
            block,
            kind: StrViewKind::Bytes,
        })
    );
}

#[test]
fn a_call_must_match_its_targets_parameter_count() {
    let source = "func identity(value: i32) -> i32 { value } \
                  func main() -> i32 { identity(1) }";
    let (fixture, hir, mir) = lower_clean(source);
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    let statement = body
        .blocks
        .iter_mut()
        .flat_map(|block| block.statements.iter_mut())
        .find(|statement| {
            matches!(
                statement.kind,
                StatementKind::Assign {
                    rvalue: Rvalue::Call { .. },
                    ..
                }
            )
        })
        .expect("main calls identity");
    let StatementKind::Assign {
        rvalue: Rvalue::Call { target, .. },
        ..
    } = &mut statement.kind
    else {
        unreachable!();
    };
    let instance = target.instance();
    *target = CallTarget::Direct {
        instance,
        parameters: 2,
    };
    assert_eq!(
        body.validate_with(&hir),
        Err(ValidationError::CallArityMismatch {
            args: 1,
            parameters: 2
        })
    );
}

#[test]
fn an_aggregate_must_carry_the_declared_type_arguments() {
    let source = "struct Wrapper<T> { value: T } \
                  func main() -> i32 { \
                      let wrapper = Wrapper { value: 1 }; \
                      wrapper.value \
                  }";
    let (fixture, hir, mir) = lower_clean(source);
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    let statement = body
        .blocks
        .iter_mut()
        .flat_map(|block| block.statements.iter_mut())
        .find(|statement| {
            matches!(
                statement.kind,
                StatementKind::Assign {
                    rvalue: Rvalue::Aggregate { .. },
                    ..
                }
            )
        })
        .expect("main builds a struct");
    let StatementKind::Assign {
        rvalue: Rvalue::Aggregate { kind, .. },
        ..
    } = &mut statement.kind
    else {
        unreachable!();
    };
    let AggregateKind::Struct { adt, args } = kind else {
        panic!("expected a struct aggregate, got {kind:?}");
    };
    let adt = *adt;
    *args = hir.ty_ctxt().empty_substs();
    assert_eq!(
        body.validate_with(&hir),
        Err(ValidationError::AggregateArgsMismatch {
            adt,
            expected: 1,
            found: 0
        })
    );
}

#[test]
fn a_symbolic_call_must_carry_the_flattened_type_arguments() {
    let source = "struct Circle { radius: i32 } \
                  trait Shape { func area(self: *Self) -> i32 { 7 } } \
                  impl Shape for Circle {} \
                  func main() -> i32 { \
                      let circle = Circle { radius: 1 }; \
                      circle.area() \
                  }";
    let (fixture, hir, mir) = lower_clean(source);
    let mut body = function(&fixture, &hir, &mir, "main").clone();
    let statement = body
        .blocks
        .iter_mut()
        .flat_map(|block| block.statements.iter_mut())
        .find(|statement| {
            matches!(
                statement.kind,
                StatementKind::Assign {
                    rvalue: Rvalue::Call { .. },
                    ..
                }
            )
        })
        .expect("main calls the default method");
    let StatementKind::Assign {
        rvalue: Rvalue::Call { target, .. },
        ..
    } = &mut statement.kind
    else {
        unreachable!();
    };
    let CallTarget::TraitMethod {
        instance,
        parameters,
    } = *target
    else {
        panic!("expected a symbolic target, got {target:?}");
    };
    *target = CallTarget::TraitMethod {
        instance: Instance::new(instance.def, hir.ty_ctxt().empty_substs()),
        parameters,
    };
    assert_eq!(
        body.validate_with(&hir),
        Err(ValidationError::CallInstanceArityMismatch {
            def: instance.def,
            expected: 1,
            found: 0
        })
    );
}

#[test]
fn a_type_parameter_receiver_requires_a_symbolic_call() {
    let source = "trait Take { func take(self: Self) -> i32; } \
                  func take_one<T: Take>(value: T) -> i32 { value.take() }";
    let (fixture, hir, mir) = lower_clean(source);
    let mut body = function(&fixture, &hir, &mir, "take_one").clone();
    let statement = body
        .blocks
        .iter_mut()
        .flat_map(|block| block.statements.iter_mut())
        .find(|statement| {
            matches!(
                statement.kind,
                StatementKind::Assign {
                    rvalue: Rvalue::Call { .. },
                    ..
                }
            )
        })
        .expect("take_one calls the bound method");
    let StatementKind::Assign {
        rvalue: Rvalue::Call { target, .. },
        ..
    } = &mut statement.kind
    else {
        unreachable!();
    };
    let CallTarget::TraitMethod {
        instance,
        parameters,
    } = *target
    else {
        panic!("expected a symbolic target, got {target:?}");
    };
    *target = CallTarget::Direct {
        instance,
        parameters,
    };
    assert!(matches!(
        body.validate_with(&hir),
        Err(ValidationError::ParamReceiverOnDirectCall { .. })
    ));
}
