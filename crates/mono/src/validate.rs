//! Validation of collected instances.

use std::fmt;

use seal_const_eval::{AllocId, AllocSource, ConstProgram, ConstValue};
use seal_def::DefId;
use seal_hir::Hir;
use seal_layout::VtableLayout;
use seal_mir::{
    AggregateKind, CallTarget, Constant, IntrinsicReceiver, Operand, Rvalue, StatementKind,
    TerminatorKind, ValidationError,
};
use seal_span::Span;
use seal_ty::{Instance, TyCtxt, TyId, TyKind};

use crate::vtable::{VtableKey, key_of};
use crate::{MonoProgram, MonoVTable, VtableId};

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
    /// A coercion has no vtable in the collected program.
    MissingVtable {
        /// The instance containing the coercion.
        instance: Instance,
        /// The coercion's span.
        span: Span,
    },
    /// A drop site or a vtable needs glue the collector did not produce.
    MissingDropGlue {
        /// The type that needs a destructor.
        ty: TyId,
    },
    /// A coercion does not have a concrete trait-object key.
    MalformedTraitObject {
        /// The instance containing the coercion.
        instance: Instance,
        /// The coercion's span.
        span: Span,
    },
    /// A vtable's concrete type or trait argument still has a parameter.
    VtableParameter {
        /// The broken vtable.
        vtable: VtableId,
    },
    /// A vtable's concrete type or trait argument has an inference variable.
    VtableInferenceVariable {
        /// The broken vtable.
        vtable: VtableId,
    },
    /// A vtable's concrete type is unsized or is itself a trait object.
    VtableConcreteNotSized {
        /// The broken vtable.
        vtable: VtableId,
    },
    /// A vtable's trait reference does not name its concrete type as `Self`.
    VtableSelfMismatch {
        /// The broken vtable.
        vtable: VtableId,
    },
    /// A vtable's layout could not be rebuilt.
    VtableLayout {
        /// The broken vtable.
        vtable: VtableId,
    },
    /// A vtable has the wrong number of method entries.
    VtableEntryCount {
        /// The broken vtable.
        vtable: VtableId,
        /// The layout's method count.
        expected: usize,
        /// The collected entry count.
        found: usize,
    },
    /// A collected static has no evaluated data.
    MissingStatic {
        /// The static definition.
        def: DefId,
    },
    /// A collected const allocation does not exist in the evaluated program.
    MissingConstAllocation {
        /// The missing allocation.
        alloc: AllocId,
    },
    /// A constant trait object names a pair without a collected vtable.
    MissingConstVtable {
        /// The concrete type the trait object erases.
        concrete: seal_ty::TyId,
        /// The principal trait reference.
        trait_ref: seal_ty::TraitRef,
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
            Self::MissingVtable { instance, span } => write!(
                f,
                "instance {instance:?} coerces to a trait object without a vtable at {span:?}"
            ),
            Self::MissingDropGlue { ty } => {
                write!(f, "drop glue for {ty:?} was never collected")
            }
            Self::MalformedTraitObject { instance, span } => write!(
                f,
                "instance {instance:?} has a malformed trait-object coercion at {span:?}"
            ),
            Self::VtableParameter { vtable } => {
                write!(f, "vtable {vtable:?} still contains a type parameter")
            }
            Self::VtableInferenceVariable { vtable } => {
                write!(f, "vtable {vtable:?} still contains an inference variable")
            }
            Self::VtableConcreteNotSized { vtable } => {
                write!(f, "vtable {vtable:?} has an unsized concrete type")
            }
            Self::VtableSelfMismatch { vtable } => write!(
                f,
                "vtable {vtable:?} keys its trait on a different self type"
            ),
            Self::VtableLayout { vtable } => {
                write!(f, "vtable {vtable:?} has a malformed layout")
            }
            Self::VtableEntryCount {
                vtable,
                expected,
                found,
            } => write!(
                f,
                "vtable {vtable:?} has {found} entries, but its layout needs {expected}"
            ),
            Self::MissingStatic { def } => {
                write!(f, "static {def:?} was collected without evaluated data")
            }
            Self::MissingConstAllocation { alloc } => {
                write!(
                    f,
                    "const allocation {alloc:?} was not produced by evaluation"
                )
            }
            Self::MissingConstVtable {
                concrete,
                trait_ref,
            } => write!(
                f,
                "constant trait object {concrete:?} / {trait_ref:?} has no collected vtable"
            ),
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
                match &statement.kind {
                    StatementKind::Assign { rvalue, .. } => {
                        check_rvalue(tcx, instance, program, statement.span, rvalue)?;
                    }
                    StatementKind::Drop { place, .. } => {
                        check_drop_site(hir, body, program, place)?;
                    }
                    StatementKind::DropRemaining { place, from } => {
                        check_drop_site(hir, body, program, place)?;
                        check_operand(tcx, instance, from)?;
                    }
                }
            }
            match &block.terminator.kind {
                TerminatorKind::SwitchInt { discriminant, .. } => {
                    check_operand(tcx, instance, discriminant)?;
                }
                TerminatorKind::PanicValue { message } => {
                    check_operand(tcx, instance, message)?;
                }
                TerminatorKind::Exit { code } => check_operand(tcx, instance, code)?,
                TerminatorKind::Goto(_)
                | TerminatorKind::Return
                | TerminatorKind::Panic { .. }
                | TerminatorKind::Unreachable => {}
            }
        }
        Ok(())
    }
}

/// Requires a collected glue for a drop site whose type needs one.
///
/// A drop of a trivially disposable type has nothing to call and needs no
/// glue. Skipping those is what lets a specialized body keep the drop site of a
/// generic parameter that resolved to such a type.
fn check_drop_site(
    hir: &Hir,
    body: &seal_mir::Body,
    program: &MonoProgram,
    place: &seal_mir::Place,
) -> Result<(), MonoValidationError> {
    let ty = seal_mir::place_ty(hir, body, place);
    if hir.ty_ctxt().needs_drop(ty) && program.drop_glue_id(ty).is_none() {
        return Err(MonoValidationError::MissingDropGlue { ty });
    }
    Ok(())
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
        Rvalue::Cast {
            operand, from, to, ..
        } => {
            check_operand(tcx, instance, operand)?;
            check_type(tcx, instance, *from)?;
            check_type(tcx, instance, *to)
        }
        Rvalue::MakeTraitObject {
            source,
            source_ty,
            target_ty,
        } => {
            check_operand(tcx, instance, source)?;
            check_type(tcx, instance, *source_ty)?;
            check_type(tcx, instance, *target_ty)?;
            let key = key_of(tcx, *source_ty, *target_ty)
                .ok_or(MonoValidationError::MalformedTraitObject { instance, span })?;
            if program.vtable_id(key).is_none() {
                return Err(MonoValidationError::MissingVtable { instance, span });
            }
            Ok(())
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
        Rvalue::OwnedView { source, ty, .. } => {
            check_operand(tcx, instance, source)?;
            check_type(tcx, instance, *ty)
        }
        Rvalue::SliceRef { start, end, ty, .. } | Rvalue::StrView { start, end, ty, .. } => {
            check_type(tcx, instance, *ty)?;
            for operand in start.iter().chain(end.iter()) {
                check_operand(tcx, instance, operand)?;
            }
            Ok(())
        }
        Rvalue::Len { slice } => check_operand(tcx, instance, slice),
        Rvalue::TypeInfo {
            ty, arg, result_ty, ..
        } => {
            check_type(tcx, instance, *ty)?;
            check_type(tcx, instance, *result_ty)?;
            if let Some(arg) = arg {
                check_operand(tcx, instance, arg)?;
            }
            Ok(())
        }
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
        Rvalue::IndirectCall { callee, args, .. } => {
            check_operand(tcx, instance, callee)?;
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
                CallTarget::TraitMethod { .. } | CallTarget::FnTrait { .. } => {
                    return Err(MonoValidationError::SymbolicCall { instance, span });
                }
                CallTarget::Extern { .. } => {}
                CallTarget::FnPtr { ty, .. } => {
                    check_type(tcx, instance, ty)?;
                }
                CallTarget::Closure { instance: closure } => {
                    if program.id_of(closure).is_none() {
                        return Err(MonoValidationError::UnknownInstance { instance: closure });
                    }
                    for argument in tcx.substs_args(closure.args) {
                        check_type(tcx, instance, argument)?;
                    }
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
        Constant::Int { ty, .. }
        | Constant::Float { ty, .. }
        | Constant::Str { ty, .. }
        | Constant::CStr { ty, .. } => check_type(tcx, instance, *ty),
        Constant::FunctionAddr {
            instance: function,
            ty,
        }
        | Constant::ClosureFnPtr {
            instance: function,
            ty,
        } => {
            check_type(tcx, instance, *ty)?;
            for argument in tcx.substs_args(function.args) {
                check_type(tcx, instance, argument)?;
            }
            Ok(())
        }
        Constant::Bool(_) | Constant::Char(_) | Constant::Unit => Ok(()),
    }
}

/// Checks one collected vtable and its method instances.
pub(crate) fn validate_vtable(
    hir: &Hir,
    program: &MonoProgram,
    id: VtableId,
    table: &MonoVTable,
) -> Result<(), MonoValidationError> {
    let tcx = hir.ty_ctxt();
    check_vtable_type(tcx, id, table.key.concrete)?;
    if matches!(tcx.kind(table.key.concrete), TyKind::Dyn { .. })
        || !tcx.is_sized(table.key.concrete)
    {
        return Err(MonoValidationError::VtableConcreteNotSized { vtable: id });
    }
    if table.key.trait_ref.self_ty != table.key.concrete {
        return Err(MonoValidationError::VtableSelfMismatch { vtable: id });
    }
    if program.drop_glue_id(table.key.concrete).is_none() {
        return Err(MonoValidationError::MissingDropGlue {
            ty: table.key.concrete,
        });
    }
    for argument in tcx.substs_args(table.key.trait_ref.args) {
        check_vtable_type(tcx, id, argument)?;
    }
    let layout = VtableLayout::new(tcx, table.key.trait_ref)
        .map_err(|_| MonoValidationError::VtableLayout { vtable: id })?;
    let expected = layout.method_count();
    if table.entries.len() != expected {
        return Err(MonoValidationError::VtableEntryCount {
            vtable: id,
            expected,
            found: table.entries.len(),
        });
    }
    for entry in &table.entries {
        if program.id_of(*entry).is_none() {
            return Err(MonoValidationError::UnknownInstance { instance: *entry });
        }
    }
    Ok(())
}

/// Rejects a vtable type that is not fully concrete.
fn check_vtable_type(tcx: &TyCtxt, vtable: VtableId, ty: TyId) -> Result<(), MonoValidationError> {
    match tcx.kind(ty) {
        TyKind::Param(_) => Err(MonoValidationError::VtableParameter { vtable }),
        TyKind::Infer(_) => Err(MonoValidationError::VtableInferenceVariable { vtable }),
        TyKind::Ref { pointee, .. } => check_vtable_type(tcx, vtable, pointee),
        TyKind::Slice { element } | TyKind::Array { element, .. } => {
            check_vtable_type(tcx, vtable, element)
        }
        TyKind::Tuple(tuple) => {
            for element in tcx.tuple_elements(tuple) {
                check_vtable_type(tcx, vtable, element)?;
            }
            Ok(())
        }
        TyKind::Adt { args, .. } => {
            for argument in tcx.substs_args(args) {
                check_vtable_type(tcx, vtable, argument)?;
            }
            Ok(())
        }
        TyKind::Dyn { args, .. } => {
            for argument in tcx.substs_args(args) {
                check_vtable_type(tcx, vtable, argument)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Checks that global items match the evaluated constants.
pub(crate) fn validate_globals(
    program: &MonoProgram,
    constants: &ConstProgram,
) -> Result<(), MonoValidationError> {
    for def in program.statics() {
        let data = constants
            .static_data(*def)
            .ok_or(MonoValidationError::MissingStatic { def: *def })?;
        check_allocation_reference(program, constants, data.alloc)?;
        let allocation = &constants.allocations()[data.alloc];
        check_const_value(program, constants, &allocation.value)?;
    }
    for id in program.const_allocations() {
        if !allocation_exists(constants, *id) {
            return Err(MonoValidationError::MissingConstAllocation { alloc: *id });
        }
        let allocation = &constants.allocations()[*id];
        check_const_value(program, constants, &allocation.value)?;
    }
    for instance in program.instances() {
        check_body_globals(program, constants, &instance.body)?;
    }
    Ok(())
}

fn check_body_globals(
    program: &MonoProgram,
    constants: &ConstProgram,
    body: &seal_mir::Body,
) -> Result<(), MonoValidationError> {
    for block in body.blocks.iter() {
        for statement in &block.statements {
            match &statement.kind {
                StatementKind::Assign { place, rvalue } => {
                    check_place_global(program, constants, place)?;
                    check_rvalue_globals(program, constants, rvalue)?;
                }
                StatementKind::Drop { place, .. } => {
                    check_place_global(program, constants, place)?;
                }
                StatementKind::DropRemaining { place, from } => {
                    check_place_global(program, constants, place)?;
                    check_operand_global(program, constants, from)?;
                }
            }
        }
        match &block.terminator.kind {
            TerminatorKind::SwitchInt { discriminant, .. } => {
                check_operand_global(program, constants, discriminant)?;
            }
            TerminatorKind::PanicValue { message } => {
                check_operand_global(program, constants, message)?;
            }
            TerminatorKind::Exit { code } => check_operand_global(program, constants, code)?,
            TerminatorKind::Goto(_)
            | TerminatorKind::Return
            | TerminatorKind::Panic { .. }
            | TerminatorKind::Unreachable => {}
        }
    }
    Ok(())
}

fn check_rvalue_globals(
    program: &MonoProgram,
    constants: &ConstProgram,
    rvalue: &Rvalue,
) -> Result<(), MonoValidationError> {
    match rvalue {
        Rvalue::Use(operand) | Rvalue::Cast { operand, .. } => {
            check_operand_global(program, constants, operand)
        }
        Rvalue::MakeTraitObject { source, .. } => check_operand_global(program, constants, source),
        Rvalue::TraitObjectReborrow { source, place, .. } => {
            check_operand_global(program, constants, source)?;
            check_place_global(program, constants, place)
        }
        Rvalue::Ref { place, .. } => check_place_global(program, constants, place),
        Rvalue::OwnedView { owner, source, .. } => {
            check_place_global(program, constants, owner)?;
            check_operand_global(program, constants, source)
        }
        Rvalue::SliceRef {
            base, start, end, ..
        }
        | Rvalue::StrView {
            base, start, end, ..
        } => {
            check_place_global(program, constants, base)?;
            for operand in start.iter().chain(end.iter()) {
                check_operand_global(program, constants, operand)?;
            }
            Ok(())
        }
        Rvalue::Discriminant(place) => check_place_global(program, constants, place),
        Rvalue::Len { slice } => check_operand_global(program, constants, slice),
        Rvalue::TypeInfo { arg, .. } => {
            if let Some(arg) = arg {
                check_operand_global(program, constants, arg)?;
            }
            Ok(())
        }
        Rvalue::Binary { lhs, rhs, .. } => {
            check_operand_global(program, constants, lhs)?;
            check_operand_global(program, constants, rhs)
        }
        Rvalue::Unary { operand, .. } => check_operand_global(program, constants, operand),
        Rvalue::Intrinsic { receiver, args, .. } => {
            match receiver {
                IntrinsicReceiver::Place(place) => {
                    check_place_global(program, constants, place)?;
                }
                IntrinsicReceiver::Value(operand) => {
                    check_operand_global(program, constants, operand)?;
                }
            }
            for arg in args {
                check_operand_global(program, constants, arg)?;
            }
            Ok(())
        }
        Rvalue::DynamicCall { receiver, args, .. } => {
            check_operand_global(program, constants, receiver)?;
            for arg in args {
                check_operand_global(program, constants, arg)?;
            }
            Ok(())
        }
        Rvalue::Call { args, .. } => {
            for arg in args {
                check_operand_global(program, constants, arg)?;
            }
            Ok(())
        }
        Rvalue::IndirectCall { callee, args, .. } => {
            check_operand_global(program, constants, callee)?;
            for arg in args {
                check_operand_global(program, constants, arg)?;
            }
            Ok(())
        }
        Rvalue::Aggregate { fields, .. } => {
            for (_, operand) in fields {
                check_operand_global(program, constants, operand)?;
            }
            Ok(())
        }
        Rvalue::Tuple { elements, .. } | Rvalue::Array { elements, .. } => {
            for operand in elements {
                check_operand_global(program, constants, operand)?;
            }
            Ok(())
        }
        Rvalue::Repeat { value, .. } => check_operand_global(program, constants, value),
    }
}

fn check_operand_global(
    program: &MonoProgram,
    constants: &ConstProgram,
    operand: &Operand,
) -> Result<(), MonoValidationError> {
    match operand {
        Operand::Copy(place) | Operand::Move(place) => {
            check_place_global(program, constants, place)
        }
        Operand::Const(def) => match constants.value(*def) {
            Some(value) => check_const_value(program, constants, value),
            None => Ok(()),
        },
        Operand::Constant(_) => Ok(()),
    }
}

fn check_place_global(
    program: &MonoProgram,
    constants: &ConstProgram,
    place: &seal_mir::Place,
) -> Result<(), MonoValidationError> {
    match place.root {
        seal_mir::PlaceRoot::Local(_) => Ok(()),
        seal_mir::PlaceRoot::Static(def) => {
            if program.statics().contains(&def) || program.extern_statics().contains(&def) {
                Ok(())
            } else {
                Err(MonoValidationError::MissingStatic { def })
            }
        }
        seal_mir::PlaceRoot::Const(def) => {
            if let Some(id) = constants.const_allocation(def) {
                check_allocation_reference(program, constants, id)
            } else if let Some(value) = constants.value(def) {
                check_const_value(program, constants, value)
            } else {
                Ok(())
            }
        }
    }
}

fn allocation_exists(constants: &ConstProgram, id: AllocId) -> bool {
    (id.index() as usize) < constants.allocations().len()
}

fn check_allocation_reference(
    program: &MonoProgram,
    constants: &ConstProgram,
    id: AllocId,
) -> Result<(), MonoValidationError> {
    if !allocation_exists(constants, id) {
        return Err(MonoValidationError::MissingConstAllocation { alloc: id });
    }
    match constants.allocations()[id].source {
        AllocSource::Static(def) if !program.statics().contains(&def) => {
            Err(MonoValidationError::MissingStatic { def })
        }
        AllocSource::Static(_) => Ok(()),
        AllocSource::Const(_) | AllocSource::Promoted { .. } | AllocSource::ConstFnLocal { .. } => {
            if program.const_allocations().contains(&id) {
                Ok(())
            } else {
                Err(MonoValidationError::MissingConstAllocation { alloc: id })
            }
        }
    }
}

fn check_const_value(
    program: &MonoProgram,
    constants: &ConstProgram,
    value: &ConstValue,
) -> Result<(), MonoValidationError> {
    match value {
        ConstValue::Ref { target, .. }
        | ConstValue::Slice { target, .. }
        | ConstValue::RawPtr { target, .. }
        | ConstValue::RawSlice { target, .. } => {
            check_allocation_reference(program, constants, *target)
        }
        ConstValue::Dyn {
            data,
            concrete,
            trait_ref,
            ..
        }
        | ConstValue::RawDyn {
            data,
            concrete,
            trait_ref,
            ..
        } => {
            check_allocation_reference(program, constants, *data)?;
            let key = VtableKey {
                concrete: *concrete,
                trait_ref: *trait_ref,
            };
            if program.vtable_id(key).is_some() {
                Ok(())
            } else {
                Err(MonoValidationError::MissingConstVtable {
                    concrete: *concrete,
                    trait_ref: *trait_ref,
                })
            }
        }
        ConstValue::Tuple(elements) | ConstValue::Array(elements) => {
            for element in elements {
                check_const_value(program, constants, element)?;
            }
            Ok(())
        }
        ConstValue::Struct { fields, .. } | ConstValue::Variant { fields, .. } => {
            for (_, field) in fields {
                check_const_value(program, constants, field)?;
            }
            Ok(())
        }
        ConstValue::Int { .. }
        | ConstValue::Float { .. }
        | ConstValue::Bool(_)
        | ConstValue::Char(_)
        | ConstValue::Unit
        | ConstValue::Str { .. }
        | ConstValue::RawStr { .. }
        | ConstValue::RawStrData { .. } => Ok(()),
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
        TyKind::Adt { args, .. } | TyKind::Dyn { args, .. } => {
            for argument in tcx.substs_args(args) {
                check_type(tcx, instance, argument)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}
