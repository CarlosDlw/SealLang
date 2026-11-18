//! Validation of collected instances.

use seal_hash::FxHashMap;
use seal_index::{Idx, IndexVec};
use seal_mir::{Body, CallTarget, Rvalue, StatementKind, ValidationError};
use seal_ty::NumericCastKind;
use seal_ty::{InferTyId, Instance};

use crate::tests::support::collect_clean;
use crate::{InstanceId, MonoItem, MonoMir, MonoProgram, MonoValidationError};

fn program_with(mono: MonoMir) -> MonoProgram {
    let instance = mono.instance;
    let mut ids = FxHashMap::default();
    ids.insert(instance, InstanceId::from_usize(0));
    MonoProgram::new(
        IndexVec::from_raw(vec![mono]),
        ids,
        vec![MonoItem::Function(instance)],
        IndexVec::new(),
        FxHashMap::default(),
    )
}

fn instance(
    program: &MonoProgram,
    name: &str,
    hir: &seal_hir::Hir,
    interner: &seal_symbol::Interner,
) -> MonoMir {
    program
        .instances()
        .iter()
        .find(|instance| crate::instance_name(hir, interner, instance.instance) == name)
        .unwrap_or_else(|| panic!("no instance named `{name}`"))
        .clone()
}

#[test]
fn a_clean_program_validates() {
    let (_fixture, hir, _mir, program) = collect_clean(
        "func identity<T>(value: T) -> T { value }\
         func main() -> i32 { identity(1) }",
    );
    program.validate(&hir).expect("a clean program is valid");
}

#[test]
fn a_leftover_type_parameter_is_rejected() {
    let (fixture, hir, _mir, program) = collect_clean(
        "func identity<T>(value: T) -> T { value }\
         func main() -> i32 { identity(1) }",
    );
    let mut mono = instance(&program, "identity<i32>", &hir, fixture.interner());
    let owner = mono.instance.def;
    let parameter = hir
        .flattened_generics(owner)
        .first()
        .expect("identity is generic")
        .param;
    mono.body.locals[Body::return_local()].ty = hir.ty_ctxt().param_ty(parameter);
    let malformed = program_with(mono);
    assert_eq!(
        malformed.validate(&hir),
        Err(MonoValidationError::TypeParameter {
            instance: mono_instance(&malformed)
        })
    );
}

#[test]
fn a_leftover_inference_variable_is_rejected() {
    let (fixture, hir, _mir, program) = collect_clean(
        "func identity<T>(value: T) -> T { value }\
         func main() -> i32 { identity(1) }",
    );
    let mut mono = instance(&program, "identity<i32>", &hir, fixture.interner());
    mono.body.locals[Body::return_local()].ty = hir
        .ty_ctxt()
        .infer_ty(InferTyId::from_u32(0).expect("zero is representable"));
    let malformed = program_with(mono);
    assert!(matches!(
        malformed.validate(&hir),
        Err(MonoValidationError::Body(
            ValidationError::InferredType { .. }
        ))
    ));
}

#[test]
fn a_cast_survives_monomorphization_with_its_kind() {
    let (fixture, hir, _mir, program) = collect_clean(
        "func convert<T>(value: i32, extra: T) -> i64 { value as i64 }\
         func main() -> i64 { convert(1, 2) }",
    );
    let mono = instance(&program, "convert<i32>", &hir, fixture.interner());
    let Rvalue::Cast { kind, from, to, .. } = cast_in(&mono.body) else {
        panic!("the instance contains a cast");
    };
    assert_eq!(*kind, NumericCastKind::IntToInt);
    let text = |ty| {
        hir.ty_ctxt()
            .display_with(ty, fixture.interner())
            .to_string()
    };
    assert_eq!(text(*from), "i32");
    assert_eq!(text(*to), "i64");
}

/// Returns the first cast rvalue of a body.
fn cast_in(body: &Body) -> &Rvalue {
    body.blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| match &statement.kind {
            StatementKind::Assign {
                rvalue: rvalue @ Rvalue::Cast { .. },
                ..
            } => Some(rvalue),
            _ => None,
        })
        .expect("the body contains a cast")
}

#[test]
fn a_cast_with_a_type_parameter_endpoint_is_rejected() {
    let (fixture, hir, _mir, program) = collect_clean(
        "func convert<T>(value: i32, extra: T) -> i64 { value as i64 }\
         func main() -> i64 { convert(1, 2) }",
    );
    let mut mono = instance(&program, "convert<i32>", &hir, fixture.interner());
    let owner = mono.instance.def;
    let parameter = hir
        .flattened_generics(owner)
        .first()
        .expect("convert is generic")
        .param;
    let param_ty = hir.ty_ctxt().param_ty(parameter);
    let mut found = false;
    for statement in mono
        .body
        .blocks
        .iter_mut()
        .flat_map(|block| block.statements.iter_mut())
    {
        if let StatementKind::Assign {
            rvalue: Rvalue::Cast { from, .. },
            ..
        } = &mut statement.kind
        {
            *from = param_ty;
            found = true;
        }
    }
    assert!(found, "the instance contains a cast");
    let malformed = program_with(mono);
    assert!(matches!(
        malformed.validate(&hir),
        Err(MonoValidationError::Body(
            ValidationError::CastTypeNotConcrete { .. }
        ))
    ));
}

#[test]
fn a_symbolic_call_is_rejected() {
    let (fixture, hir, _mir, program) = collect_clean(
        "func identity<T>(value: T) -> T { value }\
         func main() -> i32 { identity(1) }",
    );
    let mut mono = instance(&program, "main", &hir, fixture.interner());
    let target = first_call_target(&mut mono);
    let CallTarget::Direct {
        instance,
        parameters,
    } = *target
    else {
        panic!("main calls directly");
    };
    *target = CallTarget::TraitMethod {
        instance,
        parameters,
    };
    let malformed = program_with(mono);
    assert!(matches!(
        malformed.validate(&hir),
        Err(MonoValidationError::SymbolicCall { .. })
    ));
}

#[test]
fn an_unknown_callee_is_rejected() {
    let (fixture, hir, _mir, program) = collect_clean(
        "func identity<T>(value: T) -> T { value }\
         func main() -> i32 { identity(1) }",
    );
    let mut mono = instance(&program, "main", &hir, fixture.interner());
    let target = first_call_target(&mut mono);
    let CallTarget::Direct {
        instance,
        parameters,
    } = *target
    else {
        panic!("main calls directly");
    };
    *target = CallTarget::Direct {
        instance: Instance::new(instance.def, hir.ty_ctxt().empty_substs()),
        parameters,
    };
    let malformed = program_with(mono);
    assert!(matches!(
        malformed.validate(&hir),
        Err(MonoValidationError::UnknownInstance { .. })
    ));
}

#[test]
fn a_mismatched_argument_list_is_rejected() {
    let (fixture, hir, _mir, program) = collect_clean(
        "func identity<T>(value: T) -> T { value }\
         func main() -> i32 { identity(1) }",
    );
    let mut mono = instance(&program, "main", &hir, fixture.interner());
    let target = first_call_target(&mut mono);
    let CallTarget::Direct {
        instance,
        parameters,
    } = *target
    else {
        panic!("main calls directly");
    };
    *target = CallTarget::Direct {
        instance,
        parameters: parameters + 1,
    };
    let malformed = program_with(mono);
    assert!(matches!(
        malformed.validate(&hir),
        Err(MonoValidationError::Body(
            ValidationError::CallArityMismatch { .. }
        ))
    ));
}

/// Returns the first call target in a mutable body.
fn first_call_target(mono: &mut MonoMir) -> &mut CallTarget {
    for block in &mut mono.body.blocks {
        for statement in &mut block.statements {
            let StatementKind::Assign { rvalue, .. } = &mut statement.kind;
            if let Rvalue::Call { target, .. } = rvalue {
                return target;
            }
        }
    }
    panic!("the body has no call")
}

/// Returns the only instance in a constructed program.
fn mono_instance(program: &MonoProgram) -> Instance {
    program
        .instances()
        .iter()
        .next()
        .expect("the program has one instance")
        .instance
}
