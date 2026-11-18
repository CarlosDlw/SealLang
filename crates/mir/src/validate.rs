//! Structural validation for MIR.

use std::fmt;

use seal_def::{DefId, DefKind};
use seal_hir::Hir;
use seal_layout::VtableLayout;
use seal_ty::{RefKind, TraitRef, TyId, TyKind};

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
    /// A trait-object conversion does not start from a thin pointer.
    TraitObjectSourceNotThin {
        /// The block containing the conversion.
        block: BasicBlockId,
    },
    /// A trait-object conversion does not produce a trait-object pointer.
    TraitObjectTargetNotObject {
        /// The block containing the conversion.
        block: BasicBlockId,
    },
    /// A trait-object conversion changes sharedness of the pointer.
    TraitObjectKindMismatch {
        /// The block containing the conversion.
        block: BasicBlockId,
    },
    /// An operand's type differs from the type recorded on its rvalue.
    RvalueOperandTypeMismatch {
        /// The block containing the rvalue.
        block: BasicBlockId,
    },
    /// A dyn reborrow does not preserve the trait reference.
    TraitObjectReborrowMismatch {
        /// The block containing the reborrow.
        block: BasicBlockId,
    },
    /// A dyn reborrow loans a place that is not the target pointee.
    TraitObjectReborrowPlaceMismatch {
        /// The block containing the reborrow.
        block: BasicBlockId,
    },
    /// A dynamic call receiver is not a trait-object pointer.
    DynamicReceiverNotObject {
        /// The block containing the call.
        block: BasicBlockId,
    },
    /// A dynamic call receiver names a different principal trait.
    DynamicReceiverTraitMismatch {
        /// The block containing the call.
        block: BasicBlockId,
    },
    /// A dynamic call names a method or slot missing from its vtable.
    DynamicSlotMismatch {
        /// The block containing the call.
        block: BasicBlockId,
    },
    /// A dynamic call records a different number of argument types than values.
    DynamicArityMismatch {
        /// The number of arguments, excluding the receiver.
        args: usize,
        /// The number of recorded argument types.
        arg_tys: usize,
    },
    /// A dynamic call does not match the trait method's signature.
    DynamicSignatureMismatch {
        /// The block containing the call.
        block: BasicBlockId,
    },
    /// A move reads an unsized place by value.
    MoveFromUnsizedPlace {
        /// The block containing the move.
        block: BasicBlockId,
    },
    /// A trait-object rvalue keeps an inference-variable type.
    InferredRvalueType {
        /// The block containing the rvalue.
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
            Self::TraitObjectSourceNotThin { block } => write!(
                f,
                "block {block:?} builds a trait object from a non-thin pointer"
            ),
            Self::TraitObjectTargetNotObject { block } => write!(
                f,
                "block {block:?} coerces to a type that is not a trait-object pointer"
            ),
            Self::TraitObjectKindMismatch { block } => write!(
                f,
                "block {block:?} changes pointer sharedness in a trait-object conversion"
            ),
            Self::RvalueOperandTypeMismatch { block } => write!(
                f,
                "block {block:?} uses an operand whose type differs from the recorded rvalue type"
            ),
            Self::TraitObjectReborrowMismatch { block } => write!(
                f,
                "block {block:?} reborrows between different trait-object types"
            ),
            Self::TraitObjectReborrowPlaceMismatch { block } => write!(
                f,
                "block {block:?} reborrows a place that is not the target pointee"
            ),
            Self::DynamicReceiverNotObject { block } => write!(
                f,
                "block {block:?} dispatches on a receiver that is not a trait-object pointer"
            ),
            Self::DynamicReceiverTraitMismatch { block } => write!(
                f,
                "block {block:?} dispatches through a different principal trait"
            ),
            Self::DynamicSlotMismatch { block } => write!(
                f,
                "block {block:?} names a method or slot missing from the vtable"
            ),
            Self::DynamicArityMismatch { args, arg_tys } => write!(
                f,
                "dynamic call has {args} argument(s) but {arg_tys} argument type(s)"
            ),
            Self::DynamicSignatureMismatch { block } => write!(
                f,
                "block {block:?} has a dynamic call that does not match the trait method"
            ),
            Self::MoveFromUnsizedPlace { block } => {
                write!(f, "block {block:?} moves out of an unsized place")
            }
            Self::InferredRvalueType { block } => write!(
                f,
                "block {block:?} holds a trait-object rvalue with an inference-variable type"
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
        check_place(body, block, place.clone())?;
        if let (Some(hir), Operand::Move(_)) = (hir, operand)
            && !hir.ty_ctxt().is_sized(place_ty(hir, body, &place))
        {
            return Err(ValidationError::MoveFromUnsizedPlace { block });
        }
    }
    if let (Some(hir), Some(ty)) = (hir, constant_ty(operand))
        && contains_infer(hir, ty)
    {
        return Err(ValidationError::InferredConstant { block });
    }
    Ok(())
}

fn check_trait_object_conversion(
    body: &Body,
    block: BasicBlockId,
    source: &Operand,
    source_ty: TyId,
    target_ty: TyId,
    hir: &Hir,
) -> Result<(), ValidationError> {
    let tcx = hir.ty_ctxt();
    if contains_infer(hir, source_ty) || contains_infer(hir, target_ty) {
        return Err(ValidationError::InferredRvalueType { block });
    }
    let Some((source_kind, pointee)) = tcx.ref_parts(source_ty) else {
        return Err(ValidationError::TraitObjectSourceNotThin { block });
    };
    if !tcx.is_sized(pointee) || matches!(tcx.kind(pointee), TyKind::Dyn { .. }) {
        return Err(ValidationError::TraitObjectSourceNotThin { block });
    }
    if operand_ty(tcx, body, source).is_some_and(|ty| ty != source_ty) {
        return Err(ValidationError::RvalueOperandTypeMismatch { block });
    }
    let Some((target_kind, ..)) = dyn_pointer(tcx, target_ty) else {
        return Err(ValidationError::TraitObjectTargetNotObject { block });
    };
    if source_kind != target_kind {
        return Err(ValidationError::TraitObjectKindMismatch { block });
    }
    Ok(())
}

fn check_trait_object_reborrow(
    body: &Body,
    block: BasicBlockId,
    source: &Operand,
    place: &Place,
    from: TyId,
    to: TyId,
    hir: &Hir,
) -> Result<(), ValidationError> {
    let tcx = hir.ty_ctxt();
    if contains_infer(hir, from) || contains_infer(hir, to) {
        return Err(ValidationError::InferredRvalueType { block });
    }
    if operand_ty(tcx, body, source).is_some_and(|ty| ty != from) {
        return Err(ValidationError::RvalueOperandTypeMismatch { block });
    }
    let Some((from_kind, from_object, from_ref)) = dyn_pointer(tcx, from) else {
        return Err(ValidationError::TraitObjectReborrowMismatch { block });
    };
    let Some((to_kind, to_object, to_ref)) = dyn_pointer(tcx, to) else {
        return Err(ValidationError::TraitObjectReborrowMismatch { block });
    };
    if from_object != to_object
        || from_ref != to_ref
        || (from_kind == RefKind::Shared && to_kind == RefKind::Exclusive)
    {
        return Err(ValidationError::TraitObjectReborrowMismatch { block });
    }
    if place_ty(hir, body, place) != to_object {
        return Err(ValidationError::TraitObjectReborrowPlaceMismatch { block });
    }
    Ok(())
}

/// The parts of a dynamic call that validation compares with the trait layout.
struct DynamicCallParts<'a> {
    receiver: &'a Operand,
    args: &'a [Operand],
    object_trait_ref: TraitRef,
    method: DefId,
    slot: seal_layout::VtableSlot,
    arg_tys: &'a [TyId],
    ret_ty: TyId,
}

fn check_dynamic_call(
    body: &Body,
    block: BasicBlockId,
    call: DynamicCallParts<'_>,
    hir: &Hir,
) -> Result<(), ValidationError> {
    let DynamicCallParts {
        receiver,
        args,
        object_trait_ref,
        method,
        slot,
        arg_tys,
        ret_ty,
    } = call;
    let tcx = hir.ty_ctxt();
    let Some(receiver_ty) = operand_ty(tcx, body, receiver) else {
        return Err(ValidationError::DynamicReceiverNotObject { block });
    };
    let Some((_, _, receiver_ref)) = dyn_pointer(tcx, receiver_ty) else {
        return Err(ValidationError::DynamicReceiverNotObject { block });
    };
    if receiver_ref != object_trait_ref {
        return Err(ValidationError::DynamicReceiverTraitMismatch { block });
    }
    let layout = VtableLayout::new(tcx, object_trait_ref)
        .map_err(|_| ValidationError::DynamicSlotMismatch { block })?;
    let Some(entry) = layout.entries().iter().find(|entry| entry.slot == slot) else {
        return Err(ValidationError::DynamicSlotMismatch { block });
    };
    if entry.method() != Some(method) || layout.slot_of(method) != Some(slot) {
        return Err(ValidationError::DynamicSlotMismatch { block });
    }
    let Some(declaring_trait) = entry.declaring_trait() else {
        return Err(ValidationError::DynamicSlotMismatch { block });
    };
    let Some(definition) = hir.trait_definition(declaring_trait.trait_def) else {
        return Err(ValidationError::DynamicSlotMismatch { block });
    };
    let Some(method_def) = definition
        .methods
        .iter()
        .find(|declared| declared.def == method)
    else {
        return Err(ValidationError::DynamicSlotMismatch { block });
    };
    if hir
        .generics(method)
        .is_some_and(|generics| !generics.params.is_empty())
    {
        return Err(ValidationError::DynamicSignatureMismatch { block });
    }
    let mut parameters = vec![declaring_trait.self_ty];
    parameters.extend(tcx.substs_args(declaring_trait.args));
    let substs = tcx.substs(&parameters);
    let expected_params = &method_def.signature.params;
    if expected_params.len() != arg_tys.len() {
        return Err(ValidationError::DynamicSignatureMismatch { block });
    }
    for (expected, found) in expected_params.iter().zip(arg_tys) {
        if tcx.substitute(*expected, substs) != *found {
            return Err(ValidationError::DynamicSignatureMismatch { block });
        }
    }
    if tcx.substitute(method_def.signature.ret, substs) != ret_ty {
        return Err(ValidationError::DynamicSignatureMismatch { block });
    }
    for (expected, argument) in arg_tys.iter().zip(args) {
        if operand_ty(tcx, body, argument).is_some_and(|ty| ty != *expected) {
            return Err(ValidationError::RvalueOperandTypeMismatch { block });
        }
    }
    Ok(())
}

fn dyn_pointer(tcx: &seal_ty::TyCtxt, ty: TyId) -> Option<(RefKind, TyId, TraitRef)> {
    let (kind, pointee) = tcx.ref_parts(ty)?;
    if !matches!(tcx.kind(pointee), TyKind::Dyn { .. }) {
        return None;
    }
    let (trait_def, args) = tcx.dyn_parts(pointee)?;
    Some((kind, pointee, TraitRef::new(trait_def, pointee, args)))
}

fn operand_ty(tcx: &seal_ty::TyCtxt, body: &Body, operand: &Operand) -> Option<TyId> {
    match operand {
        Operand::Copy(place) | Operand::Move(place) => Some(place_ty_mir(tcx, body, place)),
        Operand::Constant(constant) => constant_ty(&Operand::Constant(*constant)),
    }
}

fn place_ty(hir: &Hir, body: &Body, place: &Place) -> TyId {
    place_ty_mir(hir.ty_ctxt(), body, place)
}

fn place_ty_mir(tcx: &seal_ty::TyCtxt, body: &Body, place: &Place) -> TyId {
    let mut ty = body.local(place.local).ty;
    for projection in &place.projections {
        ty = match projection {
            crate::place::Projection::Deref => match tcx.kind(ty) {
                TyKind::Ref { pointee, .. } => pointee,
                _ => ty,
            },
            crate::place::Projection::Field(field) => match tcx.kind(ty) {
                TyKind::Adt { adt, args } => tcx.field_ty(adt, args, *field).unwrap_or(ty),
                _ => ty,
            },
            crate::place::Projection::VariantField { variant, field } => match tcx.kind(ty) {
                TyKind::Adt { adt, args } => tcx
                    .variant_field_ty(adt, args, *variant, *field)
                    .unwrap_or(ty),
                _ => ty,
            },
            crate::place::Projection::TupleField(index) => match tcx.kind(ty) {
                TyKind::Tuple(tuple) => tcx.tuple_element(tuple, *index as usize).unwrap_or(ty),
                _ => ty,
            },
            crate::place::Projection::Index { .. } => match tcx.kind(ty) {
                TyKind::Array { element, .. } | TyKind::Slice { element } => element,
                _ => ty,
            },
        };
    }
    ty
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
        Rvalue::MakeTraitObject {
            source,
            source_ty,
            target_ty,
        } => {
            check_operand(body, block, source, hir)?;
            if let Some(hir) = hir {
                check_trait_object_conversion(body, block, source, *source_ty, *target_ty, hir)?;
            }
            Ok(())
        }
        Rvalue::TraitObjectReborrow {
            source,
            place,
            from,
            to,
        } => {
            check_operand(body, block, source, hir)?;
            check_place(body, block, place.clone())?;
            if let Some(hir) = hir {
                check_trait_object_reborrow(body, block, source, place, *from, *to, hir)?;
            }
            Ok(())
        }
        Rvalue::DynamicCall {
            object_trait_ref,
            method,
            slot,
            receiver,
            args,
            arg_spans,
            arg_tys,
            ret_ty,
        } => {
            if args.len() + 1 != arg_spans.len() {
                return Err(ValidationError::CallSpansMismatch {
                    args: args.len() + 1,
                    spans: arg_spans.len(),
                });
            }
            if args.len() != arg_tys.len() {
                return Err(ValidationError::DynamicArityMismatch {
                    args: args.len(),
                    arg_tys: arg_tys.len(),
                });
            }
            check_operand(body, block, receiver, hir)?;
            for arg in args {
                check_operand(body, block, arg, hir)?;
            }
            if let Some(hir) = hir {
                check_dynamic_call(
                    body,
                    block,
                    DynamicCallParts {
                        receiver,
                        args,
                        object_trait_ref: *object_trait_ref,
                        method: *method,
                        slot: *slot,
                        arg_tys,
                        ret_ty: *ret_ty,
                    },
                    hir,
                )?;
            }
            Ok(())
        }
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
        TyKind::Adt { args, .. } | TyKind::Dyn { args, .. } => tcx
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
