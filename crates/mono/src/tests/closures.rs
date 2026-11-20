//! Closure collection, substitution, and function-trait resolution.

use seal_def::{DefId, DefKind};
use seal_hir::Hir;
use seal_mir::{BodyKind, CallTarget, Constant, LocalKind, Operand, Rvalue, StatementKind};
use seal_ty::TyKind;

use crate::collector::Limits;
use crate::tests::support::{Fixture, collect_clean};
use crate::{MonoMir, MonoProgram};

/// Returns `name` when it is declared with `kind`.
fn def_named(fixture: &Fixture, hir: &Hir, kind: DefKind, name: &str) -> DefId {
    hir.definitions()
        .iter_enumerated()
        .find(|(_, data)| data.kind == kind && fixture.symbol(data.name) == name)
        .map(|(id, _)| id)
        .unwrap_or_else(|| panic!("no {kind:?} named `{name}`"))
}

/// Returns the only collected closure body.
fn closure_instance<'a>(hir: &Hir, program: &'a MonoProgram) -> &'a MonoMir {
    let mut closures = program
        .instances()
        .iter()
        .filter(|instance| hir.definitions().kind(instance.instance.def) == DefKind::Closure);
    let closure = closures.next().expect("the program collects a closure");
    assert!(
        closures.next().is_none(),
        "the program collects one closure"
    );
    closure
}

/// Returns the instance of `main`.
fn main_instance<'a>(fixture: &Fixture, hir: &Hir, program: &'a MonoProgram) -> &'a MonoMir {
    let main = def_named(fixture, hir, DefKind::Function, "main");
    program
        .instances()
        .iter()
        .find(|instance| instance.instance.def == main)
        .expect("main is collected")
}

fn environment_local() -> seal_mir::LocalId {
    seal_mir::LocalId::from_u32(1).expect("one is a valid local")
}

#[test]
fn a_closure_body_is_collected_with_its_environment() {
    let (fixture, hir, _mir, program) =
        collect_clean("func main() -> i32 { let f = move || 1; f() }");
    let closure = closure_instance(&hir, &program);
    assert_eq!(closure.body.kind, BodyKind::Closure);
    assert_eq!(closure.body.arg_count, 1, "the environment is an argument");
    assert_eq!(
        closure.body.local(environment_local()).kind,
        LocalKind::Arg,
        "local one is the environment"
    );

    let main = main_instance(&fixture, &hir, &program);
    let target = main
        .body
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| match &statement.kind {
            StatementKind::Assign {
                rvalue: Rvalue::Call { target, .. },
                ..
            } => Some(target),
            _ => None,
        })
        .expect("main calls the closure");
    let CallTarget::Closure { instance } = target else {
        panic!("the closure call is resolved");
    };
    assert_eq!(hir.definitions().kind(instance.def), DefKind::Closure);
    assert!(program.id_of(*instance).is_some());
    program
        .validate(&hir)
        .expect("the monomorphized program is valid");
}

#[test]
fn nested_closures_collect_both_bodies_once() {
    let (_fixture, hir, _mir, program) = collect_clean(
        "func main() -> i32 { \
             let x = 1; \
             let outer = || { let inner = || x + 1; inner() }; \
             outer() \
         }",
    );
    let closures: Vec<_> = program
        .instances()
        .iter()
        .filter(|instance| hir.definitions().kind(instance.instance.def) == DefKind::Closure)
        .collect();
    assert_eq!(closures.len(), 2, "both lifted bodies are collected");
    for closure in closures {
        assert_eq!(closure.body.kind, BodyKind::Closure);
        assert_eq!(closure.body.arg_count, 1);
        assert!(program.id_of(closure.instance).is_some());
    }
    program
        .validate(&hir)
        .expect("nested closure bodies are valid");
}

#[test]
fn a_generic_parent_substitutes_a_closure_function_pointer() {
    let (fixture, hir, _mir, program) = collect_clean(
        "func make<T>(value: T) -> i32 { let f: func(i32) -> i32 = |x| x + 1; f(1) }\
         func main() -> i32 { make(true) }",
    );
    let closure = closure_instance(&hir, &program);
    let arguments = hir.ty_ctxt().substs_args(closure.instance.args);
    assert_eq!(arguments.len(), 1);
    assert_eq!(
        hir.ty_ctxt()
            .display_with(arguments[0], fixture.interner())
            .to_string(),
        "bool"
    );
    let make = def_named(&fixture, &hir, DefKind::Function, "make");
    let make = program
        .instances()
        .iter()
        .find(|instance| instance.instance.def == make)
        .expect("the parent instance is collected");
    let constant = make
        .body
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| match &statement.kind {
            StatementKind::Assign {
                rvalue: Rvalue::Use(Operand::Constant(constant @ Constant::ClosureFnPtr { .. })),
                ..
            } => Some(constant),
            _ => None,
        })
        .expect("the generic body records the closure function pointer");
    let Constant::ClosureFnPtr { ty, .. } = constant else {
        unreachable!();
    };
    assert!(
        !hir.ty_ctxt().contains_param(*ty),
        "the pointer type is fully substituted"
    );
    program
        .validate(&hir)
        .expect("the substituted function pointer is valid");
}

#[test]
fn all_three_function_traits_resolve_a_pointer_by_value() {
    let (_fixture, hir, _mir, program) = collect_clean(
        "func add(value: i32) -> i32 { value + 1 }\
         func apply<F: Fn(i32) -> i32>(f: F, value: i32) -> i32 { f(value) }\
         func each<F: FnMut(i32) -> i32>(f: F, value: i32) -> i32 { let mut f = f; f(value) }\
         func consume<F: FnOnce(i32) -> i32>(f: F, value: i32) -> i32 { f(value) }\
         func main() -> i32 {\
             let pointer: func(i32) -> i32 = add;\
             apply(pointer, 1) + each(pointer, 2) + consume(pointer, 3)\
         }",
    );
    let resolved: Vec<_> = program
        .instances()
        .iter()
        .filter(|instance| {
            hir.definitions().kind(instance.instance.def) == DefKind::Function
                && instance
                    .body
                    .blocks
                    .iter()
                    .flat_map(|block| block.statements.iter())
                    .any(|statement| {
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
        })
        .collect();
    assert_eq!(resolved.len(), 3, "one call per function trait");
    program
        .validate(&hir)
        .expect("the function pointer calls are valid");
}

#[test]
fn an_unresolved_function_trait_reports_a_missing_impl() {
    let fixture =
        Fixture::new("func apply<F: Fn(i32) -> i32>(f: F, value: i32) -> i32 { f(value) }");
    let (hir, mir) = fixture.frontend();
    let apply = def_named(&fixture, &hir, DefKind::Function, "apply");
    let body = mir.body_of(apply).expect("apply has a body");
    let target = body
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| match &statement.kind {
            StatementKind::Assign {
                rvalue: Rvalue::Call { target, .. },
                ..
            } => Some(*target),
            _ => None,
        })
        .expect("apply calls its bound");
    let CallTarget::FnTrait {
        mut trait_ref,
        kind,
        parameters,
    } = target
    else {
        panic!("the bound call is symbolic");
    };
    trait_ref.self_ty = hir.ty_ctxt().int_ty(seal_ty::IntTy::I32);
    let result = crate::substitute::resolve_fn_trait(
        &hir,
        fixture.interner(),
        CallTarget::FnTrait {
            trait_ref,
            kind,
            parameters,
        },
        &mut [],
    );
    assert!(matches!(result, Err(crate::MonoError::MissingImpl { .. })));
}

#[test]
fn the_depth_limit_applies_to_nested_closures() {
    let fixture = Fixture::new(
        "func main() -> i32 { let outer = || { let inner = || 1; inner() }; outer() }",
    );
    let (_hir, _mir, program) = fixture.lower_with(Limits {
        depth: 1,
        instances: 4096,
        vtables: 4096,
        glues: 4096,
        glue_depth: 64,
    });
    fixture.expect("E7001", "recursion limit");
    assert!(!program.instances().is_empty());
}

#[test]
fn a_generic_parent_substitutes_the_closure_instance_arguments() {
    let (fixture, hir, _mir, program) = collect_clean(
        "func make<T>(value: T) -> i32 { let f = move || 1; f() }\
         func main() -> i32 { make(1) }",
    );
    let closure = closure_instance(&hir, &program);
    let arguments = hir.ty_ctxt().substs_args(closure.instance.args);
    assert_eq!(
        arguments.len(),
        1,
        "the closure inherits the parent's parameter"
    );
    assert_eq!(
        hir.ty_ctxt()
            .display_with(arguments[0], fixture.interner())
            .to_string(),
        "i32"
    );
    let environment = closure.body.local(environment_local());
    let TyKind::Ref { pointee, .. } = hir.ty_ctxt().kind(environment.ty) else {
        panic!("the closure is `Fn` and receives a shared environment borrow");
    };
    let TyKind::Adt { args, .. } = hir.ty_ctxt().kind(pointee) else {
        panic!("the environment points at the closure aggregate");
    };
    assert_eq!(
        hir.ty_ctxt()
            .display_with(hir.ty_ctxt().substs_args(args)[0], fixture.interner())
            .to_string(),
        "i32"
    );
    program
        .validate(&hir)
        .expect("the generic closure is valid");
}

#[test]
fn a_capture_free_closure_constant_keeps_its_body_reachable() {
    let (fixture, hir, _mir, program) =
        collect_clean("func main() -> i32 { let f: func(i32) -> i32 = |x| x + 1; f(1) }");
    let closure = closure_instance(&hir, &program);
    assert_eq!(
        closure.body.arg_count, 2,
        "the environment and the declared parameter"
    );
    let main = main_instance(&fixture, &hir, &program);
    let constant = main
        .body
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| match &statement.kind {
            StatementKind::Assign {
                rvalue: Rvalue::Use(Operand::Constant(constant @ Constant::ClosureFnPtr { .. })),
                ..
            } => Some(constant),
            _ => None,
        })
        .expect("the coercion is a closure function pointer");
    let Constant::ClosureFnPtr { instance, .. } = constant else {
        unreachable!();
    };
    assert_eq!(hir.definitions().kind(instance.def), DefKind::Closure);
    assert!(program.id_of(*instance).is_some());
    program
        .validate(&hir)
        .expect("the function pointer is valid");
}

#[test]
fn a_generic_function_trait_call_resolves_to_the_concrete_closure() {
    let (fixture, hir, _mir, program) = collect_clean(
        "func apply<F: Fn(i32) -> i32>(f: F, value: i32) -> i32 { f(value) }\
         func main() -> i32 { apply(|value: i32| value + 1, 1) }",
    );
    let apply = program
        .instances()
        .iter()
        .find(|instance| {
            hir.definitions().kind(instance.instance.def) == DefKind::Function
                && instance.body.arg_count == 2
                && instance.instance.def != def_named(&fixture, &hir, DefKind::Function, "main")
        })
        .expect("apply is collected");
    let target = apply
        .body
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter())
        .find_map(|statement| match &statement.kind {
            StatementKind::Assign {
                rvalue: Rvalue::Call { target, args, .. },
                ..
            } => Some((target, args)),
            _ => None,
        })
        .expect("apply calls its bound");
    let (target, args) = target;
    let CallTarget::Closure { instance } = target else {
        panic!("the bound call resolved to a closure, got {target:?}");
    };
    assert_eq!(hir.definitions().kind(instance.def), DefKind::Closure);
    assert_eq!(args.len(), 2);
    assert!(
        matches!(&args[0], Operand::Copy(place) if place.is_whole()),
        "`Fn` passes the environment borrow through"
    );
    assert!(program.id_of(*instance).is_some());
    program
        .validate(&hir)
        .expect("the resolved closure call is valid");
}

#[test]
fn a_function_pointer_parameter_resolves_the_receiver_to_the_pointer() {
    let (fixture, hir, _mir, program) = collect_clean(
        "func add(value: i32) -> i32 { value + 1 }\
         func apply<F: Fn(i32) -> i32>(f: F, value: i32) -> i32 { f(value) }\
         func main() -> i32 {\
             let pointer: func(i32) -> i32 = add;\
             apply(pointer, 1)\
         }",
    );
    let apply = program
        .instances()
        .iter()
        .find(|instance| {
            instance.body.arg_count == 2
                && instance.instance.def != def_named(&fixture, &hir, DefKind::Function, "main")
                && instance
                    .body
                    .blocks
                    .iter()
                    .flat_map(|block| block.statements.iter())
                    .any(|statement| {
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
        })
        .expect("apply is instantiated for the function pointer");
    assert_eq!(
        hir.definitions().kind(apply.instance.def),
        DefKind::Function
    );
    assert!(program.id_of(apply.instance).is_some());
    program.validate(&hir).expect("the pointer call is valid");
}
