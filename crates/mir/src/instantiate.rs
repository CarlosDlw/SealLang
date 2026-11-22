//! Body specialization for generic instances.
//!
//! Monomorphization and constant evaluation both need the same substitution:
//! every local type, rvalue type, and call instance in a body replaced by the
//! concrete arguments of one instance. Keeping it here prevents the two phases
//! from drifting apart.

use seal_ty::{Instance, SubstsId, TyCtxt, TyId};

use crate::body::Body;
use crate::operand::{Constant, Operand};
use crate::rvalue::{AggregateKind, CallTarget, IntrinsicReceiver, Rvalue};
use crate::statement::StatementKind;
use crate::terminator::TerminatorKind;

/// Returns `body` with every type replaced by the instance's arguments.
pub fn instantiate_body(hir: &seal_hir::Hir, body: &Body, instance: Instance) -> Body {
    let tcx = hir.ty_ctxt();
    let substs = instance.args;
    let mut specialized = body.clone();
    for declaration in &mut specialized.locals {
        declaration.ty = tcx.substitute(declaration.ty, substs);
    }
    for block in &mut specialized.blocks {
        for statement in &mut block.statements {
            match &mut statement.kind {
                StatementKind::Assign { rvalue, .. } => substitute_rvalue(tcx, substs, rvalue),
                // A place names storage, never a type: only operands carry types.
                StatementKind::Drop { .. } => {}
                StatementKind::DropRemaining { from, .. } => substitute_operand(tcx, substs, from),
            }
        }
        match &mut block.terminator.kind {
            TerminatorKind::SwitchInt { discriminant, .. } => {
                substitute_operand(tcx, substs, discriminant);
            }
            TerminatorKind::PanicValue { message } => {
                substitute_operand(tcx, substs, message);
            }
            TerminatorKind::Exit { code } => substitute_operand(tcx, substs, code),
            TerminatorKind::Goto(_)
            | TerminatorKind::Return
            | TerminatorKind::Panic { .. }
            | TerminatorKind::Unreachable => {}
        }
    }
    specialized
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
        Rvalue::SliceRef { start, end, ty, .. } | Rvalue::StrView { start, end, ty, .. } => {
            if let Some(start) = start {
                substitute_operand(tcx, substs, start);
            }
            if let Some(end) = end {
                substitute_operand(tcx, substs, end);
            }
            *ty = tcx.substitute(*ty, substs);
        }
        Rvalue::Len { slice } => substitute_operand(tcx, substs, slice),
        Rvalue::TypeInfo {
            ty, arg, result_ty, ..
        } => {
            *ty = tcx.substitute(*ty, substs);
            *result_ty = tcx.substitute(*result_ty, substs);
            if let Some(arg) = arg {
                substitute_operand(tcx, substs, arg);
            }
        }
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
        Rvalue::IndirectCall { callee, args, .. } => {
            substitute_operand(tcx, substs, callee);
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
        CallTarget::Closure { instance } => CallTarget::Closure {
            instance: Instance::new(instance.def, substitute_substs(tcx, substs, instance.args)),
        },
        CallTarget::FnTrait {
            trait_ref,
            kind,
            parameters,
        } => CallTarget::FnTrait {
            trait_ref: substitute_trait_ref(tcx, substs, trait_ref),
            kind,
            parameters: substitute_substs(tcx, substs, parameters),
        },
        // An external declaration has no type arguments to substitute.
        CallTarget::Extern {
            def,
            parameters,
            variadic,
        } => CallTarget::Extern {
            def,
            parameters,
            variadic,
        },
        // The pointer type names the signature, so it follows the instance.
        CallTarget::FnPtr { ty, receiver } => CallTarget::FnPtr {
            ty: tcx.substitute(ty, substs),
            receiver,
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
        Constant::Int { ty, .. }
        | Constant::Float { ty, .. }
        | Constant::Str { ty, .. }
        | Constant::CStr { ty, .. }
        | Constant::FunctionAddr { ty, .. }
        | Constant::ClosureFnPtr { ty, .. } => {
            *ty = tcx.substitute(*ty, substs);
        }
        Constant::Bool(_) | Constant::Char(_) | Constant::Unit => {}
    }
    match constant {
        Constant::FunctionAddr { instance, .. } | Constant::ClosureFnPtr { instance, .. } => {
            *instance = Instance::new(instance.def, substitute_substs(tcx, substs, instance.args));
        }
        _ => {}
    }
}
