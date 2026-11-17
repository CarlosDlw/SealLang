//! Entry point validation and the C `main` wrapper.

use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Linkage;
use inkwell::module::Module;
use inkwell::values::{BasicValueEnum, FunctionValue, ValueKind};
use seal_def::{DefId, DefKind};
use seal_hir::Hir;
use seal_mir::Body;
use seal_mono::MonoProgram;
use seal_symbol::PredefinedSymbol;
use seal_ty::{Instance, IntTy, TyKind};

use crate::error::{CodegenError, EntryPointError};
use crate::names;

/// Validates the program's entry point and returns its instance.
///
/// Only executables need an entry point; object and assembly output can be
/// produced for a file with no `main`. The generic check runs before looking at
/// the monomorphized body, so a generic `main` is rejected with the entry-point
/// diagnostic instead of the monomorphizer's instance diagnostic.
pub(crate) fn validate(hir: &Hir, mono: &MonoProgram) -> Result<Instance, CodegenError> {
    let main = PredefinedSymbol::Main.symbol();
    let candidates: Vec<DefId> = hir
        .definitions()
        .iter_enumerated()
        .filter(|(_, definition)| {
            definition.kind == DefKind::Function
                && definition.parent.is_none()
                && definition.name == main
        })
        .map(|(id, _)| id)
        .collect();
    let definition = match candidates.len() {
        0 => return Err(CodegenError::EntryPoint(EntryPointError::Missing)),
        1 => candidates[0],
        count => return Err(CodegenError::EntryPoint(EntryPointError::Multiple(count))),
    };
    let generics = hir.flattened_generics(definition).len();
    if generics != 0 {
        return Err(CodegenError::EntryPoint(EntryPointError::Generic(generics)));
    }
    let instance = Instance::new(definition, hir.ty_ctxt().empty_substs());
    let body = mono
        .instances()
        .iter()
        .find(|mono| mono.instance == instance)
        .map(|mono| &mono.body)
        .ok_or_else(|| {
            CodegenError::internal(
                "the entry point has no monomorphized body; collection is incomplete",
            )
        })?;
    if body.arg_count != 0 {
        return Err(CodegenError::EntryPoint(EntryPointError::Parameters(
            body.arg_count,
        )));
    }
    let returns = body.local(Body::return_local()).ty;
    match hir.ty_ctxt().kind(returns) {
        TyKind::Unit | TyKind::Int(IntTy::I32) => Ok(instance),
        other => Err(CodegenError::EntryPoint(EntryPointError::ReturnType(
            describe(other),
        ))),
    }
}

fn describe(kind: TyKind) -> String {
    match kind {
        TyKind::Bool => "bool".to_owned(),
        TyKind::Char => "char".to_owned(),
        TyKind::Str => "str".to_owned(),
        TyKind::Slice { .. } => "a slice".to_owned(),
        TyKind::Int(int) => int.as_str().to_owned(),
        TyKind::Uint(uint) => uint.as_str().to_owned(),
        TyKind::Float(float) => float.as_str().to_owned(),
        TyKind::Ref { kind, .. } => format!("{}T", kind.prefix()),
        TyKind::Array { .. } => "an array".to_owned(),
        TyKind::Tuple(_) => "a tuple".to_owned(),
        TyKind::Adt { .. } => "a struct".to_owned(),
        TyKind::Unit => "()".to_owned(),
        TyKind::Never => "!".to_owned(),
        TyKind::Error => "an error type".to_owned(),
        TyKind::Infer(_) => "an inferred type".to_owned(),
        TyKind::Param(_) => "a type parameter".to_owned(),
    }
}

/// Emits the C `main` wrapper that calls the Seal entry point.
///
/// A Seal `main` returns `i32` or `()`. The wrapper always returns `i32`, so the
/// process exit status is the language-level result and a unit `main` exits with
/// zero.
pub(crate) fn emit_wrapper<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    builder: &Builder<'ctx>,
    user_main: FunctionValue<'ctx>,
) -> Result<(), CodegenError> {
    let i32_type = context.i32_type();
    let wrapper_type = i32_type.fn_type(&[], false);
    let wrapper = module.add_function(names::ENTRY_POINT, wrapper_type, Some(Linkage::External));
    let entry = context.append_basic_block(wrapper, "entry");
    builder.position_at_end(entry);
    let call = builder
        .build_call(user_main, &[], "seal_main_result")
        .map_err(|error| {
            CodegenError::internal(format!("cannot build the entry point call: {error:?}"))
        })?;
    let status = match call.try_as_basic_value() {
        ValueKind::Basic(BasicValueEnum::IntValue(value)) => value,
        ValueKind::Basic(_) | ValueKind::Instruction(_) => i32_type.const_zero(),
    };
    builder.build_return(Some(&status)).map_err(|error| {
        CodegenError::internal(format!("cannot build the entry point return: {error:?}"))
    })?;
    Ok(())
}
