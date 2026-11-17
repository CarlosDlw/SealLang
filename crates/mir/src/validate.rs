//! Structural validation for MIR.

use std::fmt;

use seal_def::{DefId, DefKind};
use seal_hir::Hir;
use seal_ty::TyKind;

use crate::body::{Body, LocalKind};
use crate::id::{BasicBlockId, LocalId};
use crate::operand::Operand;
use crate::place::Place;
use crate::rvalue::{CallTarget, IntrinsicReceiver, Rvalue};
use crate::statement::StatementKind;
use crate::terminator::TerminatorKind;

/// A broken MIR invariant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidationError {
    /// The body has no basic blocks.
    NoBlocks,
    /// The entry block is not in the body.
    EntryOutOfRange {
        /// The recorded entry.
        entry: BasicBlockId,
        /// The number of blocks.
        blocks: usize,
    },
    /// The body has no locals at all.
    MissingReturnLocal,
    /// Local zero is not the return place.
    ReturnLocalHasWrongKind {
        /// The kind found at local zero.
        kind: LocalKind,
    },
    /// The argument count does not leave room for the return local.
    ArgCountOutOfRange {
        /// The recorded argument count.
        arg_count: usize,
        /// The number of locals.
        locals: usize,
    },
    /// A parameter local has the wrong kind.
    ArgLocalHasWrongKind {
        /// The local.
        local: LocalId,
        /// The kind found.
        kind: LocalKind,
    },
    /// An argument-kind local sits outside the argument range.
    UnexpectedArg {
        /// The local.
        local: LocalId,
    },
    /// A place names a local that is not in the body.
    PlaceOutOfRange {
        /// The block containing the place.
        block: BasicBlockId,
        /// The invalid local.
        local: LocalId,
    },
    /// A control-flow edge names a block that is not in the body.
    BlockTargetOutOfRange {
        /// The block containing the edge.
        block: BasicBlockId,
        /// The invalid target.
        target: BasicBlockId,
    },
    /// A call's argument spans do not match its operands.
    CallSpansMismatch {
        /// The number of operands.
        args: usize,
        /// The number of recorded spans.
        spans: usize,
    },
    /// A local still has an inference-variable type.
    InferredType {
        /// The local with the unresolved type.
        local: LocalId,
    },
    /// A literal still has an inference-variable type.
    InferredConstant {
        /// The block containing the literal.
        block: BasicBlockId,
    },
    /// A call passes a different number of arguments than the target expects.
    CallArityMismatch {
        /// The operands supplied.
        args: usize,
        /// The parameters the target declares.
        parameters: usize,
    },
    /// An aggregate literal supplies the wrong number of type arguments.
    AggregateArgsMismatch {
        /// The aggregate definition.
        adt: DefId,
        /// The number of parameters the declaration takes.
        expected: u32,
        /// The number of type arguments recorded on the literal.
        found: usize,
    },
    /// A known method receives a type parameter by value.
    ///
    /// Such a receiver only exists behind a bound: the implementation is
    /// unknown until monomorphization, so the call must stay symbolic.
    ParamReceiverOnDirectCall {
        /// The block containing the call.
        block: BasicBlockId,
    },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoBlocks => f.write_str("MIR body has no basic blocks"),
            Self::EntryOutOfRange { entry, blocks } => {
                write!(
                    f,
                    "entry block {entry:?} is out of range for {blocks} blocks"
                )
            }
            Self::MissingReturnLocal => f.write_str("MIR body has no return local"),
            Self::ReturnLocalHasWrongKind { kind } => {
                write!(f, "local zero has kind {kind:?}, expected the return place")
            }
            Self::ArgCountOutOfRange { arg_count, locals } => write!(
                f,
                "body declares {arg_count} arguments but has only {locals} locals"
            ),
            Self::ArgLocalHasWrongKind { local, kind } => {
                write!(f, "local {local:?} is an argument but has kind {kind:?}")
            }
            Self::UnexpectedArg { local } => {
                write!(
                    f,
                    "local {local:?} has argument kind outside the argument range"
                )
            }
            Self::PlaceOutOfRange { block, local } => {
                write!(f, "block {block:?} refers to missing local {local:?}")
            }
            Self::BlockTargetOutOfRange { block, target } => {
                write!(f, "block {block:?} jumps to missing block {target:?}")
            }
            Self::CallSpansMismatch { args, spans } => write!(
                f,
                "call has {args} argument(s) but {spans} argument span(s)"
            ),
            Self::InferredType { local } => {
                write!(f, "local {local:?} still has an inference-variable type")
            }
            Self::InferredConstant { block } => write!(
                f,
                "block {block:?} holds a literal with an inference-variable type"
            ),
            Self::CallArityMismatch { args, parameters } => write!(
                f,
                "call passes {args} argument(s) but the target takes {parameters}"
            ),
            Self::AggregateArgsMismatch {
                adt,
                expected,
                found,
            } => write!(
                f,
                "aggregate {adt:?} takes {expected} type argument(s) but {found} were recorded"
            ),
            Self::ParamReceiverOnDirectCall { block } => write!(
                f,
                "block {block:?} passes a type parameter by value to a known callee"
            ),
        }
    }
}

impl std::error::Error for ValidationError {}

/// Checks one body.
///
/// The structural invariants always apply. When `hir` is provided, the type
/// invariants apply too: no local may keep an inference variable, a call must
/// match its target's parameter count, an aggregate must carry the declared
/// number of type arguments, and a by-value type-parameter receiver may only
/// reach a symbolic trait call.
pub(crate) fn validate_body(body: &Body, hir: Option<&Hir>) -> Result<(), ValidationError> {
    if body.blocks.is_empty() {
        return Err(ValidationError::NoBlocks);
    }
    if body.entry.index() as usize >= body.blocks.len() {
        return Err(ValidationError::EntryOutOfRange {
            entry: body.entry,
            blocks: body.blocks.len(),
        });
    }
    if body.locals.is_empty() {
        return Err(ValidationError::MissingReturnLocal);
    }
    if body.arg_count >= body.locals.len() {
        return Err(ValidationError::ArgCountOutOfRange {
            arg_count: body.arg_count,
            locals: body.locals.len(),
        });
    }

    for (local, declaration) in body.locals.iter_enumerated() {
        let index = local.index() as usize;
        let kind = declaration.kind;
        if index == 0 {
            if kind != LocalKind::Return {
                return Err(ValidationError::ReturnLocalHasWrongKind { kind });
            }
        } else if index <= body.arg_count {
            if kind != LocalKind::Arg {
                return Err(ValidationError::ArgLocalHasWrongKind { local, kind });
            }
        } else if kind == LocalKind::Arg {
            return Err(ValidationError::UnexpectedArg { local });
        }
        if let Some(hir) = hir
            && contains_infer(hir, declaration.ty)
        {
            return Err(ValidationError::InferredType { local });
        }
    }

    for (block, data) in body.blocks.iter_enumerated() {
        for statement in &data.statements {
            let StatementKind::Assign { place, rvalue } = &statement.kind;
            check_place(body, block, place.clone())?;
            check_rvalue(body, block, rvalue, hir)?;
        }
        match &data.terminator.kind {
            TerminatorKind::Goto(target) => check_target(body, block, *target)?,
            TerminatorKind::SwitchInt {
                discriminant,
                targets,
                otherwise,
            } => {
                check_operand(body, block, discriminant, hir)?;
                for target in targets {
                    check_target(body, block, target.target)?;
                }
                check_target(body, block, *otherwise)?;
            }
            TerminatorKind::Return | TerminatorKind::Panic { .. } | TerminatorKind::Unreachable => {
            }
        }
    }

    Ok(())
}

fn check_place(body: &Body, block: BasicBlockId, place: Place) -> Result<(), ValidationError> {
    if place.local.index() as usize >= body.locals.len() {
        return Err(ValidationError::PlaceOutOfRange {
            block,
            local: place.local,
        });
    }
    for projection in &place.projections {
        if let crate::place::Projection::Index { local, .. } = projection
            && local.index() as usize >= body.locals.len()
        {
            return Err(ValidationError::PlaceOutOfRange {
                block,
                local: *local,
            });
        }
    }
    Ok(())
}

fn check_operand(
    body: &Body,
    block: BasicBlockId,
    operand: &Operand,
    hir: Option<&Hir>,
) -> Result<(), ValidationError> {
    if let Some(place) = operand.place() {
        check_place(body, block, place)?;
    }
    if let (Some(hir), Some(ty)) = (hir, constant_ty(operand))
        && contains_infer(hir, ty)
    {
        return Err(ValidationError::InferredConstant { block });
    }
    Ok(())
}

/// Returns the type of a literal operand, when it carries one.
fn constant_ty(operand: &Operand) -> Option<seal_ty::TyId> {
    match operand {
        Operand::Constant(
            crate::operand::Constant::Int { ty, .. }
            | crate::operand::Constant::Float { ty, .. }
            | crate::operand::Constant::Str { ty, .. },
        ) => Some(*ty),
        _ => None,
    }
}

fn check_rvalue(
    body: &Body,
    block: BasicBlockId,
    rvalue: &Rvalue,
    hir: Option<&Hir>,
) -> Result<(), ValidationError> {
    match rvalue {
        Rvalue::Use(operand) => check_operand(body, block, operand, hir),
        Rvalue::Discriminant(place) => check_place(body, block, place.clone()),
        Rvalue::Ref { place, .. } => check_place(body, block, place.clone()),
        Rvalue::SliceRef {
            base, start, end, ..
        } => {
            check_place(body, block, base.clone())?;
            if let Some(start) = start {
                check_operand(body, block, start, hir)?;
            }
            if let Some(end) = end {
                check_operand(body, block, end, hir)?;
            }
            Ok(())
        }
        Rvalue::Len { slice } => check_operand(body, block, slice, hir),
        Rvalue::Binary { lhs, rhs, .. } => {
            check_operand(body, block, lhs, hir)?;
            check_operand(body, block, rhs, hir)
        }
        Rvalue::Unary { operand, .. } => check_operand(body, block, operand, hir),
        Rvalue::Intrinsic { receiver, args, .. } => {
            match receiver {
                IntrinsicReceiver::Place(place) => check_place(body, block, place.clone())?,
                IntrinsicReceiver::Value(operand) => check_operand(body, block, operand, hir)?,
            }
            for arg in args {
                check_operand(body, block, arg, hir)?;
            }
            Ok(())
        }
        Rvalue::Call {
            target,
            args,
            arg_spans,
        } => {
            if args.len() != arg_spans.len() {
                return Err(ValidationError::CallSpansMismatch {
                    args: args.len(),
                    spans: arg_spans.len(),
                });
            }
            if let Some(hir) = hir {
                if args.len() != target.parameters() {
                    return Err(ValidationError::CallArityMismatch {
                        args: args.len(),
                        parameters: target.parameters(),
                    });
                }
                if matches!(target, CallTarget::Direct { .. })
                    && has_receiver(hir, target.instance().def)
                    && first_argument_is_param(hir, body, args)
                {
                    return Err(ValidationError::ParamReceiverOnDirectCall { block });
                }
            }
            for arg in args {
                check_operand(body, block, arg, hir)?;
            }
            Ok(())
        }
        Rvalue::Aggregate { kind, fields } => {
            if let Some(hir) = hir {
                let (adt, args) = match kind {
                    crate::rvalue::AggregateKind::Struct { adt, args } => (*adt, *args),
                    crate::rvalue::AggregateKind::Variant { adt, args, .. } => (*adt, *args),
                };
                if let Some(adt_id) = hir.adt(adt) {
                    let expected = hir.ty_ctxt().adt_generic_count(adt_id);
                    let found = hir.ty_ctxt().substs_args(args).len();
                    if found != expected as usize {
                        return Err(ValidationError::AggregateArgsMismatch {
                            adt,
                            expected,
                            found,
                        });
                    }
                }
            }
            for (_, operand) in fields {
                check_operand(body, block, operand, hir)?;
            }
            Ok(())
        }
        Rvalue::Tuple { elements, .. } | Rvalue::Array { elements, .. } => {
            for element in elements {
                check_operand(body, block, element, hir)?;
            }
            Ok(())
        }
        Rvalue::Repeat { value, .. } => check_operand(body, block, value, hir),
    }
}

/// Returns `true` when `ty` is, or contains, an inference variable.
fn contains_infer(hir: &Hir, ty: seal_ty::TyId) -> bool {
    let tcx = hir.ty_ctxt();
    match tcx.kind(ty) {
        TyKind::Infer(_) => true,
        TyKind::Ref { pointee, .. } => contains_infer(hir, pointee),
        TyKind::Slice { element } | TyKind::Array { element, .. } => contains_infer(hir, element),
        TyKind::Tuple(tuple) => tcx
            .tuple_elements(tuple)
            .iter()
            .any(|element| contains_infer(hir, *element)),
        TyKind::Adt { args, .. } => tcx
            .substs_args(args)
            .iter()
            .any(|argument| contains_infer(hir, *argument)),
        _ => false,
    }
}

/// Returns `true` when `def` is a method with a receiver.
///
/// A trait method declaration has no entry in the impl receiver registry, but a
/// direct call can never target one: bound calls stay symbolic. An impl
/// function that declares no receiver is an associated function and its first
/// argument is an ordinary parameter.
fn has_receiver(hir: &Hir, def: DefId) -> bool {
    hir.definitions().kind(def) == DefKind::TraitMethod || hir.receiver(def).is_some()
}

/// Returns `true` when the first operand is a whole local of type `Param`.
fn first_argument_is_param(hir: &Hir, body: &Body, args: &[Operand]) -> bool {
    args.first().and_then(Operand::place).is_some_and(|place| {
        place.is_whole()
            && matches!(
                hir.ty_ctxt().kind(body.local(place.local).ty),
                TyKind::Param(_)
            )
    })
}

fn check_target(
    body: &Body,
    block: BasicBlockId,
    target: BasicBlockId,
) -> Result<(), ValidationError> {
    if target.index() as usize >= body.blocks.len() {
        return Err(ValidationError::BlockTargetOutOfRange { block, target });
    }
    Ok(())
}
