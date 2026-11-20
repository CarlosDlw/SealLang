//! C-ABI trampolines for Seal functions used as C callbacks.
//!
//! A Seal function is declared with Seal's internal ABI. Its scalar parameters
//! happen to match C, but an `extern struct` passed by value does not: LLVM
//! would pass the aggregate directly while a C caller uses the target's
//! aggregate classification. Casting the function's raw address would therefore
//! miscompile every callback with an aggregate parameter or result.
//!
//! Every cast instead resolves to one generated wrapper with the C signature.
//! The wrapper receives C arguments, converts them to Seal values, calls the
//! internal function, and converts the result back. The wrapper is cached by
//! `(instance, signature)`, so repeated casts share one function and repeated
//! builds produce the same symbols.

use inkwell::module::Linkage;
use inkwell::types::BasicTypeEnum;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue, ValueKind};
use seal_ty::FnSigDef;
use seal_ty::TyId;

use crate::c_abi::{CParamKind, CReturn};
use crate::error::CodegenError;
use crate::function::FunctionLowerer;
use crate::names;

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    /// Returns the callable C pointer for the address of `instance`.
    ///
    /// An external function already has a C declaration, so its pointer is used
    /// directly. A Seal function resolves to a generated C-ABI trampoline.
    pub(crate) fn function_address(
        &self,
        instance: seal_ty::Instance,
        pointer_ty: TyId,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        // A Seal function pointer calls the internal body directly: both sides
        // use the internal ABI, so no C trampoline is involved.
        if self.tcx.fn_ptr_parts(pointer_ty).is_some() {
            let internal = self.functions.get(&instance).copied().ok_or_else(|| {
                CodegenError::internal(format!(
                    "the address of {instance:?} was requested but its body was not declared"
                ))
            })?;
            return Ok(internal.as_global_value().as_pointer_value().into());
        }
        if let Some(function) = self.externals.function(instance.def) {
            return Ok(function.as_global_value().as_pointer_value().into());
        }
        let internal = self.functions.get(&instance).copied().ok_or_else(|| {
            CodegenError::internal(format!(
                "the address of {instance:?} was requested but its body was not declared"
            ))
        })?;
        let (_, pointee) = self.tcx.raw_parts(pointer_ty).ok_or_else(|| {
            CodegenError::internal("a function address has a non-pointer type".to_owned())
        })?;
        let signature_id = self.tcx.fn_sig_id(pointee).ok_or_else(|| {
            CodegenError::internal("a function address points at a non-function type".to_owned())
        })?;
        let signature = self.tcx.fn_sig(signature_id);
        let wrapper = self.callback_wrapper(instance, internal, &signature, pointer_ty)?;
        Ok(wrapper.as_global_value().as_pointer_value().into())
    }

    /// Returns the cached C-ABI wrapper for one function and signature.
    fn callback_wrapper(
        &self,
        instance: seal_ty::Instance,
        internal: FunctionValue<'ctx>,
        signature: &FnSigDef,
        pointer_ty: TyId,
    ) -> Result<FunctionValue<'ctx>, CodegenError> {
        let name = names::callback(
            seal_mono::instance_hash(self.hir, instance),
            seal_mono::type_hash(self.hir, pointer_ty),
        );
        if let Some(existing) = self.module.get_function(&name) {
            return Ok(existing);
        }

        let c_signature =
            self.mapper
                .c_signature(&signature.params, signature.ret, signature.variadic)?;
        let wrapper =
            self.module
                .add_function(&name, c_signature.function_type, Some(Linkage::Internal));
        crate::extern_decl::apply_signature_attributes(self.module, wrapper, &c_signature);

        let builder = self.context.create_builder();
        let entry = self.context.append_basic_block(wrapper, "entry");
        builder.position_at_end(entry);

        let fixed = usize::from(matches!(c_signature.ret, CReturn::Sret(_)));
        let mut arguments = Vec::with_capacity(signature.params.len());
        for (index, parameter_ty) in signature.params.iter().enumerate() {
            let position = fixed + index;
            let argument = wrapper.get_nth_param(position as u32).ok_or_else(|| {
                CodegenError::internal(format!(
                    "callback wrapper {name} is missing C parameter {position}"
                ))
            })?;
            let converted = match c_signature.params.get(position).map(|param| &param.kind) {
                Some(CParamKind::Direct) => argument,
                Some(CParamKind::Coerced(_)) => coerce(
                    &builder,
                    argument,
                    self.mapper.basic(*parameter_ty)?,
                    "callback.param",
                )?,
                Some(CParamKind::ByPointer { aggregate, .. }) => build(
                    builder.build_load(*aggregate, argument.into_pointer_value(), "callback.byval"),
                    "callback by-value load",
                )?,
                None => {
                    return Err(CodegenError::internal(format!(
                        "callback wrapper {name} has no C parameter {position}"
                    )));
                }
            };
            arguments.push(BasicMetadataValueEnum::from(converted));
        }

        let call = build(
            builder.build_call(internal, &arguments, "callback.call"),
            "callback call",
        )?;
        let returned = match c_signature.ret {
            CReturn::Void => None,
            CReturn::Direct => Some(match call.try_as_basic_value() {
                ValueKind::Basic(value) => value,
                ValueKind::Instruction(_) => {
                    return Err(CodegenError::internal(
                        "a callback's Seal function returned no LLVM value",
                    ));
                }
            }),
            CReturn::Coerced(ty) => {
                let ValueKind::Basic(value) = call.try_as_basic_value() else {
                    return Err(CodegenError::internal(
                        "a coerced callback result returned no LLVM value",
                    ));
                };
                Some(coerce(&builder, value, ty, "callback.result")?)
            }
            CReturn::Sret(_) => {
                let ValueKind::Basic(value) = call.try_as_basic_value() else {
                    return Err(CodegenError::internal(
                        "an sret callback result returned no LLVM value",
                    ));
                };
                let slot = wrapper
                    .get_nth_param(0)
                    .ok_or_else(|| {
                        CodegenError::internal(
                            "an sret callback wrapper is missing its destination".to_owned(),
                        )
                    })?
                    .into_pointer_value();
                build(builder.build_store(slot, value), "callback sret store")?;
                None
            }
        };
        let returned = returned
            .as_ref()
            .map(|value| value as &dyn inkwell::values::BasicValue<'ctx>);
        build(builder.build_return(returned), "callback return")?;
        Ok(wrapper)
    }
}

fn coerce<'ctx>(
    builder: &inkwell::builder::Builder<'ctx>,
    value: BasicValueEnum<'ctx>,
    target: BasicTypeEnum<'ctx>,
    name: &str,
) -> Result<BasicValueEnum<'ctx>, CodegenError> {
    let slot = build(builder.build_alloca(target, name), "callback coercion")?;
    build(builder.build_store(slot, value), "callback coercion store")?;
    build(
        builder.build_load(target, slot, name),
        "callback coercion load",
    )
}

fn build<T>(
    result: Result<T, inkwell::builder::BuilderError>,
    operation: &str,
) -> Result<T, CodegenError> {
    result.map_err(|error| CodegenError::internal(format!("cannot build {operation}: {error:?}")))
}
