//! Body specialization and trait-call resolution.

use seal_diagnostics::DiagCtxt;
use seal_error_codes as codes;
use seal_hir::Hir;
use seal_mir::{
    AggregateKind, Body, CallTarget, Constant, IntrinsicReceiver, Operand, Rvalue, StatementKind,
    TerminatorKind,
};
use seal_span::Span;
use seal_symbol::Interner;
use seal_ty::{Bound, Instance, SubstsId, TemplateUnifier, TraitImplDef, TyCtxt, TyId};

use crate::error::MonoError;
use crate::vtable::{self, VtableKey};

/// One call discovered in a specialized body.
pub(crate) struct ChildCall {
    /// The callee instance with concrete arguments.
    pub(crate) instance: Instance,
    /// The call's source span, for limits and missing-impl diagnostics.
    pub(crate) span: Span,
}

/// One discovery event in a specialized body, in statement order.
pub(crate) enum BodyEvent {
    /// A call whose callee must be collected.
    Call(ChildCall),
    /// A trait-object coercion that needs a vtable.
    Vtable {
        /// The concrete type and principal trait.
        key: VtableKey,
        /// The coercion's source span.
        span: Span,
    },
}

/// Returns `body` with every type replaced by the instance's arguments.
pub(crate) fn instantiate_body(hir: &Hir, body: &Body, instance: Instance) -> Body {
    let tcx = hir.ty_ctxt();
    let substs = instance.args;
    let mut specialized = body.clone();
    for declaration in &mut specialized.locals {
        declaration.ty = tcx.substitute(declaration.ty, substs);
    }
    for block in &mut specialized.blocks {
        for statement in &mut block.statements {
            let StatementKind::Assign { rvalue, .. } = &mut statement.kind;
            substitute_rvalue(tcx, substs, rvalue);
        }
        if let TerminatorKind::SwitchInt { discriminant, .. } = &mut block.terminator.kind {
            substitute_operand(tcx, substs, discriminant);
        }
    }
    specialized
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
            let StatementKind::Assign { rvalue, .. } = &mut statement.kind;
            match rvalue {
                Rvalue::Call { target, .. } => match *target {
                    CallTarget::Direct { instance, .. } => {
                        events.push(BodyEvent::Call(ChildCall { instance, span }));
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
        return Err(MonoError::MissingMethod {
            trait_name,
            method: interner.resolve(method_name).to_owned(),
        });
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
    check_bounds(hir, interner, &implementation, substs, &impl_arguments)?;
    Ok(Instance::new(function, substs))
}

/// Verifies the selected impl's bounds for the concrete arguments.
///
/// Type checking already proves these at every call site; this pass refuses to
/// continue with an instance whose contract is not met instead of generating
/// code that assumes it.
fn check_bounds(
    hir: &Hir,
    interner: &Interner,
    implementation: &TraitImplDef,
    substs: SubstsId,
    arguments: &[TyId],
) -> Result<(), MonoError> {
    let tcx = hir.ty_ctxt();
    for (parameter, argument) in implementation.generics.params.iter().zip(arguments) {
        for bound in &parameter.bounds {
            match bound {
                Bound::Copy => {
                    if !tcx.is_copy(*argument) {
                        return Err(MonoError::MissingBound {
                            implementation: display_def(hir, interner, implementation.def),
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
                            implementation: display_def(hir, interner, implementation.def),
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

fn substitute_rvalue(tcx: &TyCtxt, substs: SubstsId, rvalue: &mut Rvalue) {
    match rvalue {
        Rvalue::Use(operand) => substitute_operand(tcx, substs, operand),
        Rvalue::Cast {
            operand, from, to, ..
        } => {
            substitute_operand(tcx, substs, operand);
            *from = tcx.substitute(*from, substs);
            *to = tcx.substitute(*to, substs);
        }
        Rvalue::MakeTraitObject {
            source,
            source_ty,
            target_ty,
        } => {
            substitute_operand(tcx, substs, source);
            *source_ty = tcx.substitute(*source_ty, substs);
            *target_ty = tcx.substitute(*target_ty, substs);
        }
        Rvalue::TraitObjectReborrow {
            source, from, to, ..
        } => {
            substitute_operand(tcx, substs, source);
            *from = tcx.substitute(*from, substs);
            *to = tcx.substitute(*to, substs);
        }
        Rvalue::DynamicCall {
            object_trait_ref,
            receiver,
            args,
            arg_tys,
            ret_ty,
            ..
        } => {
            *object_trait_ref = substitute_trait_ref(tcx, substs, *object_trait_ref);
            substitute_operand(tcx, substs, receiver);
            for argument in args {
                substitute_operand(tcx, substs, argument);
            }
            for ty in arg_tys {
                *ty = tcx.substitute(*ty, substs);
            }
            *ret_ty = tcx.substitute(*ret_ty, substs);
        }
        Rvalue::Ref { .. } | Rvalue::Discriminant(_) => {}
        Rvalue::SliceRef { start, end, ty, .. } => {
            if let Some(start) = start {
                substitute_operand(tcx, substs, start);
            }
            if let Some(end) = end {
                substitute_operand(tcx, substs, end);
            }
            *ty = tcx.substitute(*ty, substs);
        }
        Rvalue::Len { slice } => substitute_operand(tcx, substs, slice),
        Rvalue::Binary { lhs, rhs, .. } => {
            substitute_operand(tcx, substs, lhs);
            substitute_operand(tcx, substs, rhs);
        }
        Rvalue::Unary { operand, .. } => substitute_operand(tcx, substs, operand),
        Rvalue::Intrinsic { receiver, args, .. } => {
            if let IntrinsicReceiver::Value(operand) = receiver {
                substitute_operand(tcx, substs, operand);
            }
            for argument in args {
                substitute_operand(tcx, substs, argument);
            }
        }
        Rvalue::Call { target, args, .. } => {
            *target = substitute_target(tcx, substs, *target);
            for argument in args {
                substitute_operand(tcx, substs, argument);
            }
        }
        Rvalue::Aggregate { kind, fields } => {
            *kind = match *kind {
                AggregateKind::Struct { adt, args } => AggregateKind::Struct {
                    adt,
                    args: substitute_substs(tcx, substs, args),
                },
                AggregateKind::Variant { adt, variant, args } => AggregateKind::Variant {
                    adt,
                    variant,
                    args: substitute_substs(tcx, substs, args),
                },
            };
            for (_, field) in fields {
                substitute_operand(tcx, substs, field);
            }
        }
        Rvalue::Tuple { elements, ty } | Rvalue::Array { elements, ty } => {
            for element in elements {
                substitute_operand(tcx, substs, element);
            }
            *ty = tcx.substitute(*ty, substs);
        }
        Rvalue::Repeat { value, ty, .. } => {
            substitute_operand(tcx, substs, value);
            *ty = tcx.substitute(*ty, substs);
        }
    }
}

fn substitute_trait_ref(
    tcx: &TyCtxt,
    substs: SubstsId,
    target: seal_ty::TraitRef,
) -> seal_ty::TraitRef {
    seal_ty::TraitRef::new(
        target.trait_def,
        tcx.substitute(target.self_ty, substs),
        substitute_substs(tcx, substs, target.args),
    )
}

fn substitute_target(tcx: &TyCtxt, substs: SubstsId, target: CallTarget) -> CallTarget {
    match target {
        CallTarget::Direct {
            instance,
            parameters,
        } => CallTarget::Direct {
            instance: Instance::new(instance.def, substitute_substs(tcx, substs, instance.args)),
            parameters,
        },
        CallTarget::TraitMethod {
            instance,
            parameters,
        } => CallTarget::TraitMethod {
            instance: Instance::new(instance.def, substitute_substs(tcx, substs, instance.args)),
            parameters,
        },
    }
}

fn substitute_substs(tcx: &TyCtxt, substs: SubstsId, target: SubstsId) -> SubstsId {
    let arguments: Vec<TyId> = tcx
        .substs_args(target)
        .iter()
        .map(|argument| tcx.substitute(*argument, substs))
        .collect();
    tcx.substs(&arguments)
}

fn substitute_operand(tcx: &TyCtxt, substs: SubstsId, operand: &mut Operand) {
    let Operand::Constant(constant) = operand else {
        return;
    };
    match constant {
        Constant::Int { ty, .. } | Constant::Float { ty, .. } | Constant::Str { ty, .. } => {
            *ty = tcx.substitute(*ty, substs);
        }
        Constant::Bool(_) | Constant::Char(_) | Constant::Unit => {}
    }
}

fn display_def(hir: &Hir, interner: &Interner, def: seal_def::DefId) -> String {
    interner.resolve(hir.definitions().name(def)).to_owned()
}

fn display_type(tcx: &TyCtxt, interner: &Interner, ty: TyId) -> String {
    tcx.display_with(ty, interner).to_string()
}

fn report(dcx: &DiagCtxt, span: Span, error: &MonoError) {
    dcx.error(error.to_string())
        .code(codes::E7002.diag_code())
        .primary(span, "this call needs a concrete implementation")
        .emit();
}
