//! Validation of collected instances.

use std::fmt;

use seal_hir::Hir;
use seal_mir::{
    AggregateKind, CallTarget, Constant, IntrinsicReceiver, Operand, Rvalue, StatementKind,
    TerminatorKind, ValidationError,
};
use seal_span::Span;
use seal_ty::{Instance, TyId, TyKind};

use crate::MonoProgram;

/// A broken only-monomorphic invariant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MonoValidationError {
    /// The structural or generic MIR rules rejected the body.
    Body(ValidationError),
    /// A type still contains a declared parameter.
    TypeParameter {
        /// The instance whose body contains it.
        instance: Instance,
    },
    /// A type still contains an inference variable.
    InferenceVariable {
        /// The instance whose body contains it.
        instance: Instance,
    },
    /// A call target still names a trait method.
    SymbolicCall {
        /// The instance whose body contains it.
        instance: Instance,
        /// The call's span.
        span: Span,
    },
    /// A call names an instance the collector did not produce.
    UnknownInstance {
        /// The missing callee.
        instance: Instance,
    },
}

impl fmt::Display for MonoValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Body(error) => write!(f, "the specialized body is invalid: {error}"),
            Self::TypeParameter { instance } => write!(
                f,
                "instance {instance:?} still contains a declared type parameter"
            ),
            Self::InferenceVariable { instance } => write!(
                f,
                "instance {instance:?} still contains an inference variable"
            ),
            Self::SymbolicCall { instance, span } => write!(
                f,
                "instance {instance:?} still contains a symbolic trait call at {span:?}"
            ),
            Self::UnknownInstance { instance } => {
                write!(f, "instance {instance:?} was never collected")
            }
        }
    }
}

impl std::error::Error for MonoValidationError {}

impl crate::MonoMir {
    /// Checks this instance against the program that produced it.
    pub fn validate(&self, hir: &Hir, program: &MonoProgram) -> Result<(), MonoValidationError> {
        let instance = self.instance;
        self.body
            .validate_with(hir)
            .map_err(MonoValidationError::Body)?;
        let body = &self.body;
        let tcx = hir.ty_ctxt();
        for declaration in &body.locals {
            check_type(tcx, instance, declaration.ty)?;
        }
        for block in &body.blocks {
            for statement in &block.statements {
                let StatementKind::Assign { rvalue, .. } = &statement.kind;
                check_rvalue(tcx, instance, program, statement.span, rvalue)?;
            }
            if let TerminatorKind::SwitchInt { discriminant, .. } = &block.terminator.kind {
                check_operand(tcx, instance, discriminant)?;
            }
        }
        Ok(())
    }
}

fn check_rvalue(
    tcx: &seal_ty::TyCtxt,
    instance: Instance,
    program: &MonoProgram,
    span: Span,
    rvalue: &Rvalue,
) -> Result<(), MonoValidationError> {
    match rvalue {
        Rvalue::Use(operand) => check_operand(tcx, instance, operand),
        Rvalue::MakeTraitObject {
            source,
            source_ty,
            target_ty,
        } => {
            check_operand(tcx, instance, source)?;
            check_type(tcx, instance, *source_ty)?;
            check_type(tcx, instance, *target_ty)
        }
        Rvalue::TraitObjectReborrow {
            source, from, to, ..
        } => {
            check_operand(tcx, instance, source)?;
            check_type(tcx, instance, *from)?;
            check_type(tcx, instance, *to)
        }
        Rvalue::DynamicCall {
            object_trait_ref,
            receiver,
            args,
            arg_tys,
            ret_ty,
            ..
        } => {
            check_type(tcx, instance, object_trait_ref.self_ty)?;
            for argument in tcx.substs_args(object_trait_ref.args) {
                check_type(tcx, instance, argument)?;
            }
            check_operand(tcx, instance, receiver)?;
            for argument in args {
                check_operand(tcx, instance, argument)?;
            }
            for ty in arg_tys {
                check_type(tcx, instance, *ty)?;
            }
            check_type(tcx, instance, *ret_ty)
        }
        Rvalue::Ref { .. } | Rvalue::Discriminant(_) => Ok(()),
        Rvalue::SliceRef { start, end, ty, .. } => {
            check_type(tcx, instance, *ty)?;
            for operand in start.iter().chain(end.iter()) {
                check_operand(tcx, instance, operand)?;
            }
            Ok(())
        }
        Rvalue::Len { slice } => check_operand(tcx, instance, slice),
        Rvalue::Binary { lhs, rhs, .. } => {
            check_operand(tcx, instance, lhs)?;
            check_operand(tcx, instance, rhs)
        }
        Rvalue::Unary { operand, .. } => check_operand(tcx, instance, operand),
        Rvalue::Intrinsic { receiver, args, .. } => {
            if let IntrinsicReceiver::Value(operand) = receiver {
                check_operand(tcx, instance, operand)?;
            }
            for argument in args {
                check_operand(tcx, instance, argument)?;
            }
            Ok(())
        }
        Rvalue::Call { target, args, .. } => {
            match *target {
                CallTarget::Direct {
                    instance: callee, ..
                } => {
                    if program.id_of(callee).is_none() {
                        return Err(MonoValidationError::UnknownInstance { instance: callee });
                    }
                    for argument in tcx.substs_args(callee.args) {
                        check_type(tcx, instance, argument)?;
                    }
                }
                CallTarget::TraitMethod { .. } => {
                    return Err(MonoValidationError::SymbolicCall { instance, span });
                }
            }
            for argument in args {
                check_operand(tcx, instance, argument)?;
            }
            Ok(())
        }
        Rvalue::Aggregate { kind, fields } => {
            let arguments = match *kind {
                AggregateKind::Struct { args, .. } | AggregateKind::Variant { args, .. } => args,
            };
            for argument in tcx.substs_args(arguments) {
                check_type(tcx, instance, argument)?;
            }
            for (_, operand) in fields {
                check_operand(tcx, instance, operand)?;
            }
            Ok(())
        }
        Rvalue::Tuple { elements, ty } | Rvalue::Array { elements, ty } => {
            check_type(tcx, instance, *ty)?;
            for element in elements {
                check_operand(tcx, instance, element)?;
            }
            Ok(())
        }
        Rvalue::Repeat { value, ty, .. } => {
            check_type(tcx, instance, *ty)?;
            check_operand(tcx, instance, value)
        }
    }
}

fn check_operand(
    tcx: &seal_ty::TyCtxt,
    instance: Instance,
    operand: &Operand,
) -> Result<(), MonoValidationError> {
    let Operand::Constant(constant) = operand else {
        return Ok(());
    };
    match constant {
        Constant::Int { ty, .. } | Constant::Float { ty, .. } | Constant::Str { ty, .. } => {
            check_type(tcx, instance, *ty)
        }
        Constant::Bool(_) | Constant::Char(_) | Constant::Unit => Ok(()),
    }
}

/// Rejects a type that is not fully concrete.
fn check_type(
    tcx: &seal_ty::TyCtxt,
    instance: Instance,
    ty: TyId,
) -> Result<(), MonoValidationError> {
    match tcx.kind(ty) {
        TyKind::Param(_) => Err(MonoValidationError::TypeParameter { instance }),
        TyKind::Infer(_) => Err(MonoValidationError::InferenceVariable { instance }),
        TyKind::Ref { pointee, .. } => check_type(tcx, instance, pointee),
        TyKind::Slice { element } | TyKind::Array { element, .. } => {
            check_type(tcx, instance, element)
        }
        TyKind::Tuple(tuple) => {
            for element in tcx.tuple_elements(tuple) {
                check_type(tcx, instance, element)?;
            }
            Ok(())
        }
        TyKind::Adt { args, .. } => {
            for argument in tcx.substs_args(args) {
                check_type(tcx, instance, argument)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
