//! Lowering trait-object construction, reborrows, and dynamic calls.
//!
//! A trait-object pointer is the pair `{ data, vtable }`. Construction takes
//! the already-loaded thin source pointer and the vtable global selected by the
//! coercion key; it never allocates or copies the concrete value. A dynamic
//! call extracts the pair, loads one method pointer from the vtable slot, and
//! performs an indirect call with the data pointer as the receiver.

use inkwell::types::{BasicMetadataTypeEnum, BasicType};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, ValueKind};
use seal_layout::VtableLayout;
use seal_mir::Operand;
use seal_ty::{TraitRef, TyId};

use crate::error::CodegenError;
use crate::function::FunctionLowerer;

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    /// Builds `{ data, vtable }` from a concrete pointer and its vtable.
    pub(crate) fn make_trait_object(
        &self,
        source: &Operand,
        source_ty: TyId,
        target_ty: TyId,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let data = self.load_operand(source)?;
        let data = match data {
            BasicValueEnum::PointerValue(pointer) => pointer,
            other => {
                return Err(CodegenError::internal(format!(
                    "a trait-object coercion source is not a thin pointer: {other:?}"
                )));
            }
        };
        let table = self
            .vtables
            .global_for_coercion(self.tcx, source_ty, target_ty)?;
        self.build_dyn_pointer(data, table.as_pointer_value(), target_ty)
    }

    /// Copies the data and vtable pair of `source` into the target pointer.
    ///
    /// The vtable is never rebuilt: reborrowing changes only the static
    /// mutability of the reference. When the source and target types are
    /// identical, the loaded pair is returned as-is.
    pub(crate) fn reborrow_trait_object(
        &self,
        source: &Operand,
        from: TyId,
        to: TyId,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        if from == to {
            return self.load_operand(source);
        }
        // Both pointer kinds share the same two-pointer representation; only
        // the static mutability in the type differs.
        let fat = self.load_operand(source)?.into_struct_value();
        let data = self
            .build(
                self.builder.build_extract_value(fat, 0, "dyn.data"),
                "trait-object data",
            )?
            .into_pointer_value();
        let table = self
            .build(
                self.builder.build_extract_value(fat, 1, "dyn.vtable"),
                "trait-object vtable",
            )?
            .into_pointer_value();
        self.build_dyn_pointer(data, table, to)
    }

    /// Emits a dynamic call through the receiver's embedded vtable.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dynamic_call(
        &self,
        object_trait_ref: TraitRef,
        slot: seal_layout::VtableSlot,
        receiver: &Operand,
        args: &[Operand],
        arg_tys: &[TyId],
        ret_ty: TyId,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let layout = VtableLayout::new(self.tcx, object_trait_ref).map_err(|error| {
            CodegenError::internal(format!("the receiver's vtable layout is invalid: {error}"))
        })?;
        if layout.entry(slot).is_none() {
            return Err(CodegenError::internal(format!(
                "dynamic call slot {} is outside the receiver's vtable",
                slot.index()
            )));
        }
        let vtable_type = self.mapper.vtable_type(layout.entries().len())?;

        let fat = self.load_operand(receiver)?.into_struct_value();
        let data = self
            .build(
                self.builder.build_extract_value(fat, 0, "dyn.data"),
                "dynamic receiver data",
            )?
            .into_pointer_value();
        let table = self
            .build(
                self.builder.build_extract_value(fat, 1, "dyn.vtable"),
                "dynamic receiver vtable",
            )?
            .into_pointer_value();

        let slot_pointer = self.build(
            self.builder
                .build_struct_gep(vtable_type, table, slot.index(), "method slot"),
            "method slot projection",
        )?;
        let method_pointer = self
            .build(
                self.builder
                    .build_load(self.mapper.pointer(), slot_pointer, "method pointer"),
                "method pointer load",
            )?
            .into_pointer_value();

        let method_type = self.dynamic_function_type(arg_tys, ret_ty)?;
        let mut values: Vec<BasicMetadataValueEnum<'ctx>> = Vec::with_capacity(args.len() + 1);
        values.push(BasicMetadataValueEnum::from(BasicValueEnum::from(data)));
        for argument in args {
            values.push(BasicMetadataValueEnum::from(self.load_operand(argument)?));
        }
        let call = self.build(
            self.builder
                .build_indirect_call(method_type, method_pointer, &values, "dynamic_call"),
            "dynamic call",
        )?;
        match call.try_as_basic_value() {
            ValueKind::Basic(value) => Ok(value),
            ValueKind::Instruction(_) => Err(CodegenError::internal(
                "a dynamic call returned no LLVM value",
            )),
        }
    }

    /// Returns the erased indirect-call signature: a data pointer, then every
    /// recorded argument type, returning the substituted method result.
    fn dynamic_function_type(
        &self,
        arg_tys: &[TyId],
        ret_ty: TyId,
    ) -> Result<inkwell::types::FunctionType<'ctx>, CodegenError> {
        let mut parameters: Vec<BasicMetadataTypeEnum<'ctx>> =
            Vec::with_capacity(arg_tys.len() + 1);
        parameters.push(self.mapper.pointer().into());
        for argument in arg_tys {
            parameters.push(self.mapper.basic(*argument)?.into());
        }
        Ok(self.mapper.basic(ret_ty)?.fn_type(&parameters, false))
    }

    /// Inserts the data and vtable pointers into a fresh trait-object value.
    fn build_dyn_pointer(
        &self,
        data: inkwell::values::PointerValue<'ctx>,
        table: inkwell::values::PointerValue<'ctx>,
        ty: TyId,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let fat_type = self.mapper.basic(ty)?.into_struct_type();
        let mut fat = fat_type.get_undef();
        let fields: [BasicValueEnum<'ctx>; 2] = [data.into(), table.into()];
        for (index, value) in fields.into_iter().enumerate() {
            let next = self.build(
                self.builder
                    .build_insert_value(fat, value, index as u32, "dyn field"),
                "trait-object construction",
            )?;
            fat = match next {
                inkwell::values::AggregateValueEnum::StructValue(value) => value,
                other => {
                    return Err(CodegenError::internal(format!(
                        "trait-object construction produced {other:?}"
                    )));
                }
            };
        }
        Ok(fat.into())
    }
}
