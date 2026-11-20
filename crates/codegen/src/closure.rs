//! Trampolines for capture-free closures used through function pointers.
//!
//! A closure body always receives its environment as its first parameter, while
//! a Seal function pointer has the closure's declared signature without that
//! parameter. The trampoline bridges the two ABIs: it builds the empty
//! environment, calls the lifted body, and returns its result.

use inkwell::module::Linkage;
use inkwell::types::BasicMetadataTypeEnum;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue, ValueKind};
use seal_ty::{Instance, TyId};

use crate::error::CodegenError;
use crate::function::FunctionLowerer;
use crate::names;

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    /// Returns the internal function-pointer trampoline for a capture-free
    /// closure.
    ///
    /// The trampoline is cached by the closure instance. Its pointer type comes
    /// from the coercion target, so the generated call and the indirect call at
    /// the source share the same LLVM function type.
    pub(crate) fn closure_trampoline(
        &self,
        instance: Instance,
        pointer_ty: TyId,
    ) -> Result<FunctionValue<'ctx>, CodegenError> {
        let closure = self.hir.closure(instance.def).ok_or_else(|| {
            CodegenError::internal(format!(
                "closure function pointer {instance:?} has no closure definition"
            ))
        })?;
        if !closure.captures.is_empty() {
            return Err(CodegenError::internal(
                "a closure with captures reached a function-pointer trampoline",
            ));
        }
        let definition = self.tcx.fn_ptr_parts(pointer_ty).ok_or_else(|| {
            CodegenError::internal(
                "a closure function pointer does not have a Seal function type".to_owned(),
            )
        })?;
        let parent = self
            .hir
            .definitions()
            .get(instance.def)
            .parent
            .map(|parent| {
                self.interner
                    .resolve(self.hir.definitions().get(parent).name)
            })
            .unwrap_or("closure");
        let name = names::closure(parent, seal_mono::instance_hash(self.hir, instance));
        if let Some(existing) = self.module.get_function(&name) {
            return Ok(existing);
        }

        let function_type = self
            .mapper
            .function_type(&definition.params, definition.ret)?;
        let trampoline = self
            .module
            .add_function(&name, function_type, Some(Linkage::Internal));
        let builder = self.context.create_builder();
        let entry = self.context.append_basic_block(trampoline, "entry");
        builder.position_at_end(entry);

        let internal = self.functions.get(&instance).copied().ok_or_else(|| {
            CodegenError::internal(format!(
                "closure trampoline target {instance:?} was not declared"
            ))
        })?;
        let environment_ty = self.mapper.adt(closure.adt, instance.args)?;
        let environment_parameter = internal
            .get_type()
            .get_param_types()
            .first()
            .copied()
            .ok_or_else(|| {
                CodegenError::internal(format!(
                    "closure body {instance:?} has no environment parameter"
                ))
            })?;
        let environment: BasicValueEnum<'ctx> =
            if matches!(environment_parameter, BasicMetadataTypeEnum::PointerType(_)) {
                let storage = builder
                    .build_alloca(environment_ty, "closure.environment")
                    .map_err(|error| {
                        CodegenError::internal(format!(
                            "cannot allocate a closure environment: {error:?}"
                        ))
                    })?;
                storage.into()
            } else {
                environment_ty.const_zero().into()
            };

        let mut arguments = Vec::with_capacity(definition.params.len() + 1);
        arguments.push(BasicMetadataValueEnum::from(environment));
        for index in 0..definition.params.len() {
            let argument = trampoline.get_nth_param(index as u32).ok_or_else(|| {
                CodegenError::internal(format!(
                    "closure trampoline {name} is missing parameter {index}"
                ))
            })?;
            arguments.push(BasicMetadataValueEnum::from(argument));
        }
        let call = builder
            .build_call(internal, &arguments, "closure.call")
            .map_err(|error| {
                CodegenError::internal(format!("cannot call a closure body: {error:?}"))
            })?;
        let value = match call.try_as_basic_value() {
            ValueKind::Basic(value) => value,
            ValueKind::Instruction(_) => self.mapper.empty_type().const_zero().into(),
        };
        builder.build_return(Some(&value)).map_err(|error| {
            CodegenError::internal(format!("cannot return a closure: {error:?}"))
        })?;
        Ok(trampoline)
    }
}
