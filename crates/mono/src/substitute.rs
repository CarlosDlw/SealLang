//! Body specialization and trait-call resolution.

use seal_diagnostics::DiagCtxt;
use seal_error_codes as codes;
use seal_hir::Hir;
use seal_mir::{AggregateKind, Body, CallTarget, Operand, Rvalue, StatementKind};

use seal_span::Span;
use seal_symbol::Interner;
use seal_ty::{Bound, Instance, SubstsId, TemplateUnifier, TraitImplDef, TyCtxt, TyId, TyKind};

use crate::error::MonoError;
use crate::vtable::{self, VtableKey};

/// One call discovered in a specialized body.
#[derive(Clone, Copy, Debug)]
pub struct ChildCall {
    /// The callee instance with concrete arguments.
    pub instance: Instance,
    /// The call's source span, for limits and missing-impl diagnostics.
    pub span: Span,
}

/// One discovery event in a specialized body, in statement order.
#[derive(Clone, Copy, Debug)]
pub enum BodyEvent {
    /// A call whose callee must be collected.
    Call(ChildCall),
    /// A closure environment whose lifted body must be collected.
    Closure(ChildCall),
    /// A trait-object coercion that needs a vtable.
    Vtable {
        /// The concrete type and principal trait.
        key: VtableKey,
        /// The coercion's source span.
        span: Span,
    },
}

/// One body with every symbolic call resolved, plus its discovery events.
///
/// This is the cacheable unit of monomorphization: substitution and trait-call
/// resolution depend on the generic MIR body and the instance's concrete type
/// arguments, not on how many other instances the program reaches. The
/// collector still walks the graph, but a persistent caller can reuse this
/// value for every unchanged instance.
#[derive(Clone, Debug)]
pub struct SpecializedBody {
    /// The body with every type and call substituted.
    pub body: Body,
    /// The callees, closures, and vtables the body discovers.
    pub events: Vec<BodyEvent>,
}

/// Specializes one generic MIR body for a concrete instance.
pub fn specialize(
    hir: &Hir,
    dcx: &DiagCtxt,
    interner: &Interner,
    body: &Body,
    instance: Instance,
) -> SpecializedBody {
    let specialized = seal_mir::instantiate_body(hir, body, instance);
    let (body, events) = resolve_body(hir, dcx, interner, specialized);
    SpecializedBody { body, events }
}

/// Resolves symbolic calls and records vtable requests in statement order.
///
/// A symbolic call whose implementation cannot be selected is reported and left
/// symbolic; the caller stops the pipeline, and validation would reject it if it
/// reached a later phase. A malformed trait-object coercion is reported here too,
/// so a later phase never has to guess why a vtable is missing.
pub(crate) fn resolve_body(
    hir: &Hir,
    dcx: &DiagCtxt,
    interner: &Interner,
    mut body: Body,
) -> (Body, Vec<BodyEvent>) {
    let tcx = hir.ty_ctxt();
    let mut events = Vec::new();
    for block in &mut body.blocks {
        for statement in &mut block.statements {
            let span = statement.span;
            let StatementKind::Assign { rvalue, .. } = &mut statement.kind else {
                // A drop discovers no call: drop glue is built per type by code
                // generation, and an array drop only names a range operand.
                continue;
            };
            match rvalue {
                Rvalue::Call { target, args, .. } => match *target {
                    CallTarget::Direct { instance, .. } => {
                        events.push(BodyEvent::Call(ChildCall { instance, span }));
                    }
                    CallTarget::Extern { .. } => {}
                    // The callee is a pointer value, not an instance: the
                    // constant that produced it already enqueued its body.
                    CallTarget::FnPtr { .. } => {}
                    CallTarget::Closure { instance } => {
                        events.push(BodyEvent::Call(ChildCall { instance, span }));
                    }
                    CallTarget::FnTrait { .. } => {
                        match resolve_fn_trait(hir, interner, *target, args.as_mut_slice()) {
                            Ok(ResolvedFnTrait::Closure(instance)) => {
                                *target = CallTarget::Closure { instance };
                                events.push(BodyEvent::Call(ChildCall { instance, span }));
                            }
                            Ok(ResolvedFnTrait::FnPtr { ty }) => {
                                *target = CallTarget::FnPtr { ty, receiver: None };
                            }
                            Err(error) => report(dcx, span, &error),
                        }
                    }
                    CallTarget::TraitMethod {
                        instance,
                        parameters,
                    } => match resolve_trait_instance(hir, interner, instance) {
                        Ok(concrete) => {
                            *target = CallTarget::Direct {
                                instance: concrete,
                                parameters,
                            };
                            events.push(BodyEvent::Call(ChildCall {
                                instance: concrete,
                                span,
                            }));
                        }
                        Err(error) => report(dcx, span, &error),
                    },
                },
                Rvalue::Aggregate {
                    kind: AggregateKind::Struct { adt, args },
                    ..
                } => {
                    let closure = hir.adt(*adt).and_then(|adt| hir.closure_for_adt(adt));
                    if let Some(closure) = closure {
                        events.push(BodyEvent::Closure(ChildCall {
                            instance: Instance::new(closure, *args),
                            span,
                        }));
                    }
                }
                Rvalue::MakeTraitObject {
                    source_ty,
                    target_ty,
                    ..
                } => match vtable::key_of(tcx, *source_ty, *target_ty) {
                    Some(key) => events.push(BodyEvent::Vtable { key, span }),
                    None => vtable::report(
                        dcx,
                        span,
                        &MonoError::VtableLayout {
                            detail: format!(
                                "the coercion from `{}` to `{}` has no concrete trait-object key",
                                display_type(tcx, interner, *source_ty),
                                display_type(tcx, interner, *target_ty)
                            ),
                        },
                    ),
                },
                _ => {}
            }
        }
    }
    (body, events)
}

/// Selects the impl method instance for one symbolic trait call.
///
/// `instance.args` are flattened `[Self, trait args..., method args...]`, as
/// type checking recorded them for the trait declaration.
pub(crate) fn resolve_trait_instance(
    hir: &Hir,
    interner: &Interner,
    instance: Instance,
) -> Result<Instance, MonoError> {
    let tcx = hir.ty_ctxt();
    let Some(trait_def) = hir.definitions().get(instance.def).parent else {
        return Err(MonoError::MissingMethod {
            trait_name: "<unknown>".to_owned(),
            method: display_def(hir, interner, instance.def),
        });
    };
    let trait_name = display_def(hir, interner, trait_def);
    let Some(definition) = hir.trait_definition(trait_def) else {
        return Err(MonoError::MissingMethod {
            trait_name,
            method: display_def(hir, interner, instance.def),
        });
    };
    let trait_own = definition.generics.params.len().saturating_sub(1);
    let arguments = tcx.substs_args(instance.args);
    let Some(self_ty) = arguments.first().copied() else {
        return Err(MonoError::MissingMethod {
            trait_name,
            method: display_def(hir, interner, instance.def),
        });
    };
    let Some(trait_arguments) = arguments.get(1..1 + trait_own) else {
        return Err(MonoError::MissingMethod {
            trait_name,
            method: display_def(hir, interner, instance.def),
        });
    };
    let method_own = hir
        .generics(instance.def)
        .map(|generics| generics.params.len())
        .unwrap_or(0);
    let Some(method_arguments) = arguments.get(1 + trait_own..) else {
        return Err(MonoError::MissingMethod {
            trait_name,
            method: display_def(hir, interner, instance.def),
        });
    };
    if method_arguments.len() != method_own {
        return Err(MonoError::MissingMethod {
            trait_name,
            method: display_def(hir, interner, instance.def),
        });
    }

    let method_name = hir.definitions().name(instance.def);
    let mut matches: Vec<(TraitImplDef, Vec<TyId>)> = Vec::new();
    for implementation in hir.trait_impls() {
        if implementation.trait_ref.trait_def != trait_def {
            continue;
        }
        let mut unifier = TemplateUnifier::new(tcx);
        if !unifier.unify(implementation.trait_ref.self_ty, self_ty) {
            continue;
        }
        let pattern_arguments = tcx.substs_args(implementation.trait_ref.args);
        if pattern_arguments.len() != trait_arguments.len() {
            continue;
        }
        let matches_every_argument = pattern_arguments
            .into_iter()
            .zip(trait_arguments.iter().copied())
            .all(|(pattern, argument)| unifier.unify(pattern, argument));
        if !matches_every_argument {
            continue;
        }
        let matched = unifier.into_match();
        let mut impl_arguments = Vec::with_capacity(implementation.generics.params.len());
        let mut unresolved = None;
        for parameter in &implementation.generics.params {
            match matched.left().get(&parameter.param).copied() {
                Some(argument) => impl_arguments.push(argument),
                None => {
                    unresolved = Some(parameter);
                    break;
                }
            }
        }
        if let Some(parameter) = unresolved {
            return Err(MonoError::UnconstrainedParameter {
                implementation: display_def(hir, interner, implementation.def),
                parameter: interner.resolve(parameter.name).to_owned(),
            });
        }
        matches.push((implementation, impl_arguments));
    }

    if matches.is_empty() {
        return Err(MonoError::MissingImpl {
            trait_name,
            self_type: display_type(tcx, interner, self_ty),
        });
    }
    if matches.len() > 1 {
        return Err(MonoError::AmbiguousImpl {
            trait_name,
            self_type: display_type(tcx, interner, self_ty),
        });
    }
    let (implementation, impl_arguments) = matches.pop().expect("one match remains");
    let Some(function) = implementation.method(method_name) else {
        return resolve_default_method(
            hir,
            interner,
            instance,
            &implementation,
            method_arguments,
            &trait_name,
            method_name,
        );
    };
    let function_own = hir
        .generics(function)
        .map(|generics| generics.params.len())
        .unwrap_or(0);
    if function_own != method_own {
        return Err(MonoError::MissingMethod {
            trait_name,
            method: interner.resolve(method_name).to_owned(),
        });
    }
    let mut all_arguments = impl_arguments.clone();
    all_arguments.extend_from_slice(method_arguments);
    let substs = tcx.substs(&all_arguments);
    check_impl_bounds(hir, interner, &implementation, substs, &impl_arguments)?;
    Ok(Instance::new(function, substs))
}

/// Selects the trait's default body when the matching impl omits the method.
///
/// The trait method's flattened arguments are already `[Self, trait args...,
/// method args...]`, exactly the order its own body expects. The method's
/// bounds are checked here because no impl function exists to carry them.
fn resolve_default_method(
    hir: &Hir,
    interner: &Interner,
    instance: Instance,
    implementation: &TraitImplDef,
    method_arguments: &[TyId],
    trait_name: &str,
    method_name: seal_symbol::Symbol,
) -> Result<Instance, MonoError> {
    let missing = || MonoError::MissingMethod {
        trait_name: trait_name.to_owned(),
        method: interner.resolve(method_name).to_owned(),
    };
    let Some(definition) = hir.trait_definition(implementation.trait_ref.trait_def) else {
        return Err(missing());
    };
    let Some(declaration) = definition.method(method_name) else {
        return Err(missing());
    };
    if hir.body_of(declaration.def).is_none() {
        return Err(missing());
    }
    let expected = hir.flattened_generics(declaration.def).len();
    let arguments = hir.ty_ctxt().substs_args(instance.args);
    if arguments.len() != expected {
        return Err(missing());
    }
    let method_generics = hir.generics(declaration.def).ok_or_else(missing)?;
    if method_generics.params.len() != method_arguments.len() {
        return Err(missing());
    }
    check_method_bounds(
        hir,
        interner,
        declaration.def,
        &method_generics,
        instance.args,
        method_arguments,
    )?;
    Ok(Instance::new(declaration.def, instance.args))
}

/// Verifies the selected impl's bounds for the concrete arguments.
///
/// Type checking already proves these at every call site; this pass refuses to
/// continue with an instance whose contract is not met instead of generating
/// code that assumes it.
fn check_impl_bounds(
    hir: &Hir,
    interner: &Interner,
    implementation: &TraitImplDef,
    substs: SubstsId,
    arguments: &[TyId],
) -> Result<(), MonoError> {
    check_param_bounds(
        hir,
        interner,
        implementation.def,
        &implementation.generics.params,
        substs,
        arguments,
    )
}

/// Verifies the bounds declared by the trait method itself.
fn check_method_bounds(
    hir: &Hir,
    interner: &Interner,
    owner: seal_def::DefId,
    generics: &seal_ty::Generics,
    substs: SubstsId,
    arguments: &[TyId],
) -> Result<(), MonoError> {
    check_param_bounds(hir, interner, owner, &generics.params, substs, arguments)
}

/// Verifies every bound attached to the given parameters.
fn check_param_bounds(
    hir: &Hir,
    interner: &Interner,
    owner: seal_def::DefId,
    parameters: &[seal_ty::GenericParamDef],
    substs: SubstsId,
    arguments: &[TyId],
) -> Result<(), MonoError> {
    let tcx = hir.ty_ctxt();
    for (parameter, argument) in parameters.iter().zip(arguments) {
        for bound in &parameter.bounds {
            match bound {
                Bound::Copy => {
                    if !tcx.is_copy(*argument) {
                        return Err(MonoError::MissingBound {
                            implementation: display_def(hir, interner, owner),
                            argument: display_type(tcx, interner, *argument),
                        });
                    }
                }
                Bound::Trait(trait_ref) => {
                    let goal_self = tcx.substitute(trait_ref.self_ty, substs);
                    let goal_arguments: Vec<TyId> = tcx
                        .substs_args(trait_ref.args)
                        .iter()
                        .map(|argument| tcx.substitute(*argument, substs))
                        .collect();
                    if !has_impl(hir, trait_ref.trait_def, goal_self, &goal_arguments) {
                        return Err(MonoError::MissingBound {
                            implementation: display_def(hir, interner, owner),
                            argument: display_type(tcx, interner, goal_self),
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

/// Returns `true` when some impl's header matches the concrete goal.
fn has_impl(hir: &Hir, trait_def: seal_def::DefId, self_ty: TyId, arguments: &[TyId]) -> bool {
    let tcx = hir.ty_ctxt();
    hir.trait_impls().iter().any(|implementation| {
        if implementation.trait_ref.trait_def != trait_def {
            return false;
        }
        let mut unifier = TemplateUnifier::new(tcx);
        if !unifier.unify(implementation.trait_ref.self_ty, self_ty) {
            return false;
        }
        let pattern = tcx.substs_args(implementation.trait_ref.args);
        pattern.len() == arguments.len()
            && pattern
                .into_iter()
                .zip(arguments.iter().copied())
                .all(|(pattern, argument)| unifier.unify(pattern, argument))
    })
}

fn display_def(hir: &Hir, interner: &Interner, def: seal_def::DefId) -> String {
    interner.resolve(hir.definitions().name(def)).to_owned()
}

fn display_type(tcx: &TyCtxt, interner: &Interner, ty: TyId) -> String {
    tcx.display_with(ty, interner).to_string()
}

/// The concrete form selected for one symbolic function-trait call.
pub(crate) enum ResolvedFnTrait {
    Closure(Instance),
    FnPtr { ty: TyId },
}

/// Resolves a function-trait call from its concrete self type.
///
/// A closure keeps the receiver argument MIR lowering produced: a borrow for
/// `Fn`/`FnMut`, the value for `FnOnce`. A function pointer is passed by value,
/// so a borrow introduced for the generic call is dereferenced back to the
/// pointer value.
pub(crate) fn resolve_fn_trait(
    hir: &Hir,
    interner: &Interner,
    target: CallTarget,
    args: &mut [Operand],
) -> Result<ResolvedFnTrait, MonoError> {
    let tcx = hir.ty_ctxt();
    let CallTarget::FnTrait {
        trait_ref, kind, ..
    } = target
    else {
        unreachable!("only a function-trait target is resolved here");
    };
    let missing = || MonoError::MissingImpl {
        trait_name: display_def(hir, interner, trait_ref.trait_def),
        self_type: display_type(tcx, interner, trait_ref.self_ty),
    };
    match tcx.kind(trait_ref.self_ty) {
        TyKind::Adt { adt, args } => {
            let Some(closure) = hir.closure_for_adt(adt) else {
                return Err(missing());
            };
            Ok(ResolvedFnTrait::Closure(Instance::new(closure, args)))
        }
        TyKind::FnPtr(_) => {
            if kind != seal_ty::ClosureKind::FnOnce {
                let Some(place) = args.first().and_then(Operand::place) else {
                    return Err(MonoError::MissingImpl {
                        trait_name: display_def(hir, interner, trait_ref.trait_def),
                        self_type: display_type(tcx, interner, trait_ref.self_ty),
                    });
                };
                args[0] = Operand::Copy(place.deref());
            }
            Ok(ResolvedFnTrait::FnPtr {
                ty: trait_ref.self_ty,
            })
        }
        _ => Err(missing()),
    }
}

fn report(dcx: &DiagCtxt, span: Span, error: &MonoError) {
    dcx.error(error.to_string())
        .code(codes::E7002.diag_code())
        .primary(span, "this call needs a concrete implementation")
        .emit();
}
