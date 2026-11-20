//! Closure environment construction, capture access, and calls.

use seal_hir::Hir;

use crate::body::{Body, BodyKind, LocalKind};
use crate::id::LocalId;
use crate::operand::{Constant, Operand};
use crate::place::{Place, Projection};
use crate::rvalue::{AggregateKind, CallTarget, Rvalue};
use crate::validate::ValidationError;

use super::support::{assignments, definition, function, lower_clean};

/// Returns every lowered closure body.
fn closure_bodies<'a>(hir: &Hir, mir: &'a crate::Mir) -> Vec<&'a Body> {
    hir.bodies()
        .iter()
        .filter(|body| body.kind == seal_hir::BodyKind::Closure)
        .map(|body| {
            mir.body_of(body.owner)
                .unwrap_or_else(|| panic!("closure {:?} has no MIR body", body.owner))
        })
        .collect()
}

/// Returns the only closure body in the program.
fn closure_body<'a>(hir: &Hir, mir: &'a crate::Mir) -> &'a Body {
    let bodies = closure_bodies(hir, mir);
    assert_eq!(bodies.len(), 1, "the program has one closure");
    bodies[0]
}

/// Returns the closure environment aggregate built by `body`.
fn environment_aggregate<'a>(
    hir: &Hir,
    body: &'a Body,
) -> (&'a Place, &'a Vec<(seal_def::DefId, Operand)>) {
    for (place, rvalue) in assignments(body) {
        if let Rvalue::Aggregate {
            kind: AggregateKind::Struct { adt, .. },
            fields,
        } = rvalue
            && hir
                .adt(*adt)
                .and_then(|adt| hir.closure_for_adt(adt))
                .is_some()
        {
            return (place, fields);
        }
    }
    panic!("the body builds no environment aggregate");
}

/// Returns the place a capture is read from, when `body` reads one.
fn capture_read(body: &Body) -> Place {
    let environment = environment_local();
    for (_, rvalue) in assignments(body) {
        let operand = match rvalue {
            Rvalue::Use(operand) => operand,
            Rvalue::Binary { lhs, .. } => lhs,
            _ => continue,
        };
        if let Some(place) = operand.place()
            && place.root == crate::place::PlaceRoot::Local(environment)
        {
            return place;
        }
    }
    panic!("the body reads no capture");
}

fn closure_call(body: &Body) -> (&CallTarget, &[Operand]) {
    for (_, rvalue) in assignments(body) {
        if let Rvalue::Call { target, args, .. } = rvalue
            && matches!(
                target,
                CallTarget::Closure { .. } | CallTarget::FnTrait { .. }
            )
        {
            return (target, args);
        }
    }
    panic!("the body has no closure or function-trait call");
}

#[test]
fn a_move_closure_builds_an_environment_and_reads_it_through_a_borrow() {
    let (fixture, hir, mir) =
        lower_clean("func main() -> i32 { let x = 1; let f = move || x + 1; f() }");
    let main = function(&fixture, &hir, &mir, "main");
    let (_, fields) = environment_aggregate(&hir, main);
    assert_eq!(fields.len(), 1, "the environment has one capture");
    assert!(
        matches!(fields[0].1, Operand::Move(_) | Operand::Copy(_)),
        "a move capture stores the captured value"
    );

    let closure = closure_body(&hir, &mir);
    assert_eq!(closure.kind, BodyKind::Closure);
    assert_eq!(closure.arg_count, 1, "the environment is the only argument");
    let environment = closure.local(environment_local());
    assert_eq!(environment.kind, LocalKind::Arg);
    assert_eq!(environment.name, None);

    let (target, args) = closure_call(main);
    assert!(matches!(target, CallTarget::Closure { .. }));
    assert_eq!(args.len(), 1, "the call only passes the environment");
    assert!(
        matches!(&args[0], Operand::Copy(place) if place.is_whole()),
        "an `Fn` closure receives a shared borrow of its environment"
    );

    let capture = capture_read(closure);
    assert_eq!(
        capture.projections,
        vec![
            Projection::Deref,
            Projection::Field(hir.closure(closure.owner).unwrap().captures[0].field)
        ],
        "the capture is reached through the borrowed environment"
    );
}

#[test]
fn a_closure_without_an_environment_is_rejected() {
    let (fixture, hir, mir) = lower_clean("func main() -> i32 { let x = 1; let f = || x; f() }");
    let mut closure = closure_body(&hir, &mir).clone();
    closure.arg_count = 0;
    assert_eq!(
        closure.validate_with(&hir),
        Err(ValidationError::ClosureWithoutEnvironment)
    );
    let _ = fixture;
}

#[test]
fn a_closure_environment_with_the_wrong_type_is_rejected() {
    let (_fixture, hir, mir) = lower_clean("func main() -> i32 { let x = 1; let f = || x; f() }");
    let mut closure = closure_body(&hir, &mir).clone();
    closure.locals[environment_local()].ty = hir.ty_ctxt().int_ty(seal_ty::IntTy::I32);
    assert_eq!(
        closure.validate_with(&hir),
        Err(ValidationError::ClosureEnvironmentMismatch {
            local: environment_local()
        })
    );
}

#[test]
fn a_capture_projection_must_name_an_environment_field() {
    let (fixture, hir, mir) = lower_clean("func main() -> i32 { let x = 1; let f = || x; f() }");
    let mut closure = closure_body(&hir, &mir).clone();
    let invalid = definition(&fixture, &hir, seal_def::DefKind::Function, "main");
    let mut found = false;
    for block in &mut closure.blocks {
        for statement in &mut block.statements {
            let crate::statement::StatementKind::Assign { place, rvalue } = &mut statement.kind
            else {
                continue;
            };
            for projection in &mut place.projections {
                if let Projection::Field(field) = projection {
                    *field = invalid;
                    found = true;
                }
            }
            let operand = match rvalue {
                Rvalue::Use(operand) => Some(operand),
                Rvalue::Binary { lhs, .. } => Some(lhs),
                _ => None,
            };
            let place = match operand {
                Some(Operand::Copy(place) | Operand::Move(place)) => Some(place),
                _ => None,
            };
            if let Some(place) = place
                && place.root == crate::place::PlaceRoot::Local(environment_local())
            {
                for projection in &mut place.projections {
                    if let Projection::Field(field) = projection {
                        *field = invalid;
                        found = true;
                    }
                }
            }
        }
    }
    assert!(found, "the closure accesses a capture field");
    assert!(matches!(
        closure.validate_with(&hir),
        Err(ValidationError::CaptureFieldMismatch { field, .. }) if field == invalid
    ));
}

#[test]
fn a_function_trait_call_with_the_wrong_arity_is_rejected() {
    let source = "\
        func apply<F: Fn(i32) -> i32>(f: F, value: i32) -> i32 { f(value) } \
        func main() -> i32 { 0 }";
    let (fixture, hir, mir) = lower_clean(source);
    let mut apply = function(&fixture, &hir, &mir, "apply").clone();
    let mut found = false;
    for block in &mut apply.blocks {
        for statement in &mut block.statements {
            let crate::statement::StatementKind::Assign { rvalue, .. } = &mut statement.kind else {
                continue;
            };
            if let Rvalue::Call {
                target,
                args,
                arg_spans,
            } = rvalue
                && target.is_fn_trait()
            {
                args.pop();
                arg_spans.pop();
                found = true;
            }
        }
    }
    assert!(found, "the bound call is lowered");
    assert!(matches!(
        apply.validate_with(&hir),
        Err(ValidationError::FnTraitTargetMismatch { .. })
    ));
}

#[test]
fn a_closure_call_needs_its_environment_argument() {
    let (fixture, hir, mir) = lower_clean("func main() -> i32 { let x = 1; let f = || x; f() }");
    let mut main = function(&fixture, &hir, &mir, "main").clone();
    let mut found = false;
    for block in &mut main.blocks {
        for statement in &mut block.statements {
            let crate::statement::StatementKind::Assign { rvalue, .. } = &mut statement.kind else {
                continue;
            };
            if let Rvalue::Call { target, args, .. } = rvalue
                && target.is_closure()
            {
                args[0] = Operand::Constant(Constant::Int {
                    value: 0,
                    ty: hir.ty_ctxt().int_ty(seal_ty::IntTy::I32),
                });
                found = true;
            }
        }
    }
    assert!(found, "the closure call is lowered");
    assert!(matches!(
        main.validate_with(&hir),
        Err(ValidationError::ClosureTargetInvalid { .. })
    ));
}

fn environment_local() -> LocalId {
    LocalId::from_u32(1).expect("the environment is local one")
}

#[test]
fn a_mutating_closure_receives_an_exclusive_borrow() {
    let source = "func main() { let mut total = 0; let mut f = || { total = total + 1; }; f(); }";
    let (fixture, hir, mir) = lower_clean(source);
    let main = function(&fixture, &hir, &mir, "main");
    let (_, fields) = environment_aggregate(&hir, main);
    assert!(
        matches!(fields[0].1, Operand::Copy(_)),
        "a mutable capture stores a borrow, not the value"
    );

    let (_, args) = closure_call(main);
    let Operand::Copy(reference) = &args[0] else {
        panic!("an `FnMut` closure receives a borrow");
    };
    let reference = main.local(reference.expect_local());
    assert_eq!(reference.kind, LocalKind::Temp);
    let ty = fixture.ty_text(&hir, reference.ty);
    assert!(
        ty.contains("mut"),
        "the receiver is an exclusive borrow: {ty}"
    );
    let capture = capture_read(closure_body(&hir, &mir));
    assert_eq!(
        capture.projections,
        vec![
            Projection::Deref,
            Projection::Field(
                hir.closure(closure_body(&hir, &mir).owner)
                    .unwrap()
                    .captures[0]
                    .field
            ),
            Projection::Deref,
        ],
        "a mutable capture is reached through its stored borrow"
    );
}

#[test]
fn a_consuming_closure_moves_the_capture_out_of_its_environment() {
    let source = "\
        struct Token { value: i32 } \
        func consume(token: Token) {} \
        func main() { let token = Token { value: 1 }; let take = move || consume(token); take(); }";
    let (fixture, hir, mir) = lower_clean(source);
    let closure = closure_body(&hir, &mir);
    let capture = capture_read(closure);
    assert_eq!(
        capture.projections,
        vec![Projection::Field(
            hir.closure(closure.owner).unwrap().captures[0].field
        )],
        "an `FnOnce` environment stores the value directly"
    );
    assert!(
        assignments(closure).into_iter().any(|(_, rvalue)| matches!(
            rvalue,
            Rvalue::Call { target: CallTarget::Direct { .. }, args, .. }
                if matches!(args.first(), Some(Operand::Move(place)) if place.is_whole())
        )),
        "the temporary holding the capture is moved into the callee"
    );
    let _ = fixture;
}

#[test]
fn consuming_a_move_capture_forwards_the_environment_by_value() {
    let source = "\
        struct Token { value: i32 } \
        func consume(token: Token) {} \
        func main() { let token = Token { value: 1 }; let take = move || consume(token); take(); }";
    let (fixture, hir, mir) = lower_clean(source);
    let main = function(&fixture, &hir, &mir, "main");
    let (_, fields) = environment_aggregate(&hir, main);
    assert!(matches!(fields[0].1, Operand::Move(_)));

    let (target, args) = closure_call(main);
    let CallTarget::Closure { instance } = target else {
        panic!("the call names the closure");
    };
    assert_eq!(
        hir.definitions().kind(instance.def),
        seal_def::DefKind::Closure
    );
    assert_eq!(args.len(), 1);
    assert!(matches!(&args[0], Operand::Move(_)));
}

#[test]
fn a_parameterless_closure_coerces_to_a_constant_function_pointer() {
    let source = "func main() -> i32 { let f: func(i32) -> i32 = |x| x + 1; f(1) }";
    let (fixture, hir, mir) = lower_clean(source);
    let main = function(&fixture, &hir, &mir, "main");
    let constant = assignments(main)
        .into_iter()
        .find_map(|(_, rvalue)| match rvalue {
            Rvalue::Use(Operand::Constant(constant @ Constant::ClosureFnPtr { .. })) => {
                Some(constant)
            }
            _ => None,
        })
        .expect("the coercion records a closure function pointer");
    let Constant::ClosureFnPtr { instance, ty } = constant else {
        unreachable!();
    };
    assert_eq!(
        hir.definitions().kind(instance.def),
        seal_def::DefKind::Closure
    );
    assert!(
        fixture.ty_text(&hir, *ty).contains("func(i32) -> i32"),
        "the constant keeps the function pointer type"
    );
}

#[test]
fn a_generic_function_trait_call_stays_symbolic() {
    let source = "\
        func apply<F: Fn(i32) -> i32>(f: F, value: i32) -> i32 { f(value) } \
        func main() -> i32 { 0 }";
    let (fixture, hir, mir) = lower_clean(source);
    let apply = function(&fixture, &hir, &mir, "apply");
    let (target, args) = closure_call(apply);
    let CallTarget::FnTrait {
        kind, parameters, ..
    } = target
    else {
        panic!("a generic bound lowers to a symbolic function-trait call");
    };
    assert_eq!(*kind, seal_ty::ClosureKind::Fn);
    assert_eq!(hir.ty_ctxt().substs_args(*parameters).len(), 1);
    assert_eq!(args.len(), 2, "the borrow is followed by the parameter");
    assert!(matches!(&args[0], Operand::Copy(_)));
}

#[test]
fn a_nested_closure_reads_its_capture_through_the_parent_environment() {
    let source = "func main() -> i32 { let x = 1; let outer = || { let inner = || x + 1; inner() }; outer() }";
    let (_fixture, hir, mir) = lower_clean(source);
    let outer_closure = hir
        .bodies()
        .iter()
        .find(|body| {
            body.kind == seal_hir::BodyKind::Closure
                && hir
                    .definitions()
                    .get(body.owner)
                    .parent
                    .and_then(|parent| hir.closure(parent))
                    .is_none()
        })
        .expect("the outer closure exists");
    let outer = mir.body_of(outer_closure.owner).unwrap();
    let (_, fields) = environment_aggregate(&hir, outer);
    assert_eq!(fields.len(), 1, "the outer environment captures `x`");
    assert!(
        matches!(fields[0].1, Operand::Copy(_)),
        "the inner environment borrows through the parent environment"
    );

    let inner = closure_bodies(&hir, &mir)
        .into_iter()
        .find(|body| body.owner != outer.owner)
        .expect("the inner closure exists");
    let capture = capture_read(inner);
    let fields = capture
        .projections
        .iter()
        .filter(|projection| matches!(projection, Projection::Field(_)))
        .count();
    assert_eq!(
        fields, 1,
        "the inner environment points at the captured value directly"
    );
}
