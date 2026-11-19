//! Code generation for raw pointer intrinsic methods.
//!
//! Every method receives the pointer by value. Thin pointers are a single
//! address; slice, string, and trait-object pointers are two-word values whose
//! first word is the data address and whose second word is metadata. Arithmetic
//! changes only the data address, while metadata is preserved except in
//! `data()`, which discards it explicitly.

use inkwell::values::{BasicValueEnum, IntValue, PointerValue};
use seal_mir::{IntrinsicReceiver, Operand};
use seal_span::Span;
use seal_ty::{TyId, TyKind};

use crate::error::CodegenError;
use crate::function::FunctionLowerer;

/// One loaded raw pointer.
struct RawPointer<'ctx> {
    pointee: TyId,
    value: BasicValueEnum<'ctx>,
}

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    /// Emits one raw pointer intrinsic.
    pub(crate) fn raw_pointer_intrinsic(
        &self,
        intrinsic: seal_methods::IntrinsicOp,
        receiver: &IntrinsicReceiver,
        args: &[Operand],
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        use seal_methods::IntrinsicOp;
        let pointer = self.raw_pointer_receiver(receiver)?;
        match intrinsic {
            IntrinsicOp::PtrIsNull => {
                let data = self.raw_data_pointer(&pointer)?;
                let is_null = self.build(self.builder.build_is_null(data, "null"), "null check")?;
                Ok(is_null.into())
            }
            IntrinsicOp::PtrAddr | IntrinsicOp::PtrExposeAddr => {
                Ok(self.raw_address(&pointer)?.into())
            }
            IntrinsicOp::PtrLen => Ok(self.raw_length(&pointer)?.into()),
            IntrinsicOp::PtrData => Ok(self.raw_data_pointer(&pointer)?.into()),
            IntrinsicOp::PtrWithAddr => {
                let address = self.load_operand(&args[0])?.into_int_value();
                self.raw_with_addr(&pointer, address)
            }
            IntrinsicOp::PtrAdd
            | IntrinsicOp::PtrSub
            | IntrinsicOp::PtrOffset
            | IntrinsicOp::PtrByteAdd
            | IntrinsicOp::PtrByteOffset
            | IntrinsicOp::PtrWrappingAdd
            | IntrinsicOp::PtrWrappingSub
            | IntrinsicOp::PtrWrappingOffset => {
                let amount = self.load_operand(&args[0])?.into_int_value();
                self.raw_arithmetic(intrinsic, &pointer, amount)
            }
            IntrinsicOp::PtrOffsetFrom => {
                let other = self.raw_operand(&args[0])?;
                self.raw_offset_from(&pointer, &other)
            }
            IntrinsicOp::PtrRead | IntrinsicOp::PtrReadUnaligned => {
                let unaligned = intrinsic == IntrinsicOp::PtrReadUnaligned;
                self.raw_read(&pointer, unaligned)
            }
            IntrinsicOp::PtrWrite | IntrinsicOp::PtrWriteUnaligned => {
                let value = self.load_operand(&args[0])?;
                let unaligned = intrinsic == IntrinsicOp::PtrWriteUnaligned;
                self.raw_write(&pointer, value, unaligned)?;
                Ok(self.unit_value())
            }
            IntrinsicOp::PtrCopy | IntrinsicOp::PtrCopyNonOverlapping => {
                let source = self.raw_operand(&args[0])?;
                let count = self.load_operand(&args[1])?.into_int_value();
                let overlapping = intrinsic == IntrinsicOp::PtrCopy;
                self.raw_copy(&pointer, &source, count, overlapping, span)?;
                Ok(self.unit_value())
            }
            _ => Err(CodegenError::internal(
                "a non-pointer intrinsic reached raw pointer code generation",
            )),
        }
    }

    fn raw_pointer_receiver(
        &self,
        receiver: &IntrinsicReceiver,
    ) -> Result<RawPointer<'ctx>, CodegenError> {
        let IntrinsicReceiver::Value(operand) = receiver else {
            return Err(CodegenError::internal(
                "a raw pointer method has a non-value receiver",
            ));
        };
        self.raw_operand(operand)
    }

    fn raw_operand(&self, operand: &Operand) -> Result<RawPointer<'ctx>, CodegenError> {
        let ty = self.operand_type(operand)?;
        let TyKind::RawPtr { pointee, .. } = self.tcx.kind(ty) else {
            return Err(CodegenError::internal(format!(
                "raw pointer method received `{ty:?}`"
            )));
        };
        Ok(RawPointer {
            pointee,
            value: self.load_operand(operand)?,
        })
    }

    /// Returns the data address, extracting it from a two-word pointer.
    fn raw_data_pointer(
        &self,
        pointer: &RawPointer<'ctx>,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        match self.tcx.kind(pointer.pointee) {
            TyKind::Slice { .. } | TyKind::Str | TyKind::Dyn { .. } => {
                let fat = pointer.value.into_struct_value();
                Ok(self
                    .build(
                        self.builder.build_extract_value(fat, 0, "pointer data"),
                        "pointer data",
                    )?
                    .into_pointer_value())
            }
            _ => Ok(pointer.value.into_pointer_value()),
        }
    }

    /// Returns the length word of a slice or string pointer.
    fn raw_length(&self, pointer: &RawPointer<'ctx>) -> Result<IntValue<'ctx>, CodegenError> {
        match self.tcx.kind(pointer.pointee) {
            TyKind::Slice { .. } | TyKind::Str => {
                let fat = pointer.value.into_struct_value();
                Ok(self
                    .build(
                        self.builder.build_extract_value(fat, 1, "pointer length"),
                        "pointer length",
                    )?
                    .into_int_value())
            }
            other => Err(CodegenError::internal(format!(
                "`len` received a pointer to `{other:?}`"
            ))),
        }
    }

    /// Returns whether the pointer carries a metadata word.
    fn raw_is_fat(&self, pointer: &RawPointer<'ctx>) -> bool {
        matches!(
            self.tcx.kind(pointer.pointee),
            TyKind::Slice { .. } | TyKind::Str | TyKind::Dyn { .. }
        )
    }

    fn raw_address(&self, pointer: &RawPointer<'ctx>) -> Result<IntValue<'ctx>, CodegenError> {
        let data = self.raw_data_pointer(pointer)?;
        let usize = self.mapper.usize_type()?;
        self.build(
            self.builder
                .build_ptr_to_int(data, usize, "pointer address"),
            "pointer address",
        )
    }

    /// Rebuilds the pointer with a new data address and the old metadata.
    fn raw_with_addr(
        &self,
        pointer: &RawPointer<'ctx>,
        address: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let data = self.build(
            self.builder
                .build_int_to_ptr(address, self.mapper.pointer(), "pointer data"),
            "pointer data",
        )?;
        if !self.raw_is_fat(pointer) {
            return Ok(data.into());
        }
        let fat = pointer.value.into_struct_value();
        let updated = self.build(
            self.builder
                .build_insert_value(fat, data, 0, "pointer with address"),
            "pointer with address",
        )?;
        Ok(updated.into_struct_value().into())
    }

    fn raw_arithmetic(
        &self,
        intrinsic: seal_methods::IntrinsicOp,
        pointer: &RawPointer<'ctx>,
        amount: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        use seal_methods::IntrinsicOp;
        if self.raw_is_fat(pointer) {
            return Err(CodegenError::internal(
                "pointer arithmetic reached a fat pointer",
            ));
        }
        let bytewise = matches!(
            intrinsic,
            IntrinsicOp::PtrByteAdd | IntrinsicOp::PtrByteOffset
        );
        let backwards = matches!(intrinsic, IntrinsicOp::PtrSub | IntrinsicOp::PtrWrappingSub);
        let amount = if backwards {
            self.build(
                self.builder.build_int_neg(amount, "pointer retreat"),
                "pointer retreat",
            )?
        } else {
            amount
        };
        let data = pointer.value.into_pointer_value();
        let usize = self.mapper.usize_type()?;
        let base = self.build(
            self.builder.build_ptr_to_int(data, usize, "pointer base"),
            "pointer base",
        )?;
        let offset = if bytewise {
            amount
        } else {
            let size = self.mapper.layout(pointer.pointee)?.size();
            self.build(
                self.builder
                    .build_int_mul(amount, usize.const_int(size, false), "pointer offset"),
                "pointer offset",
            )?
        };
        let address = self.build(
            self.builder.build_int_add(base, offset, "pointer address"),
            "pointer address",
        )?;
        Ok(self
            .build(
                self.builder
                    .build_int_to_ptr(address, self.mapper.pointer(), "pointer result"),
                "pointer result",
            )?
            .into())
    }

    fn raw_offset_from(
        &self,
        pointer: &RawPointer<'ctx>,
        other: &RawPointer<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let data = self.raw_address(pointer)?;
        let other_data = self.raw_address(other)?;
        let difference = self.build(
            self.builder
                .build_int_sub(data, other_data, "pointer difference"),
            "pointer difference",
        )?;
        let size = self.mapper.layout(pointer.pointee)?.size();
        if size == 0 {
            let isize = self.isize_type()?;
            return Ok(isize.const_zero().into());
        }
        let size = self.mapper.usize_type()?.const_int(size, false);
        let distance = self.build(
            self.builder
                .build_int_signed_div(difference, size, "pointer distance"),
            "pointer distance",
        )?;
        Ok(distance.into())
    }

    fn isize_type(&self) -> Result<inkwell::types::IntType<'ctx>, CodegenError> {
        Ok(self
            .mapper
            .basic(self.tcx.int_ty(seal_ty::IntTy::Isize))?
            .into_int_type())
    }

    fn raw_read(
        &self,
        pointer: &RawPointer<'ctx>,
        unaligned: bool,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let data = self.raw_data_pointer(pointer)?;
        let ty = self.mapper.basic(pointer.pointee)?;
        let value = self.build(
            self.builder.build_load(ty, data, "raw read"),
            "raw pointer read",
        )?;
        if unaligned {
            self.align_last_instruction(1, "raw read")?;
        }
        Ok(value)
    }

    fn raw_write(
        &self,
        pointer: &RawPointer<'ctx>,
        value: BasicValueEnum<'ctx>,
        unaligned: bool,
    ) -> Result<(), CodegenError> {
        let data = self.raw_data_pointer(pointer)?;
        let store = self.build(self.builder.build_store(data, value), "raw pointer write")?;
        if unaligned {
            store.set_alignment(1).map_err(|error| {
                CodegenError::internal(format!("cannot align write: {error:?}"))
            })?;
        }
        Ok(())
    }

    fn raw_copy(
        &self,
        destination: &RawPointer<'ctx>,
        source: &RawPointer<'ctx>,
        count: IntValue<'ctx>,
        overlapping: bool,
        _span: Span,
    ) -> Result<(), CodegenError> {
        let element = self.mapper.layout(destination.pointee)?.size();
        let destination = self.raw_data_pointer(destination)?;
        let source = self.raw_data_pointer(source)?;
        let length = self.build(
            self.builder.build_int_mul(
                count,
                self.mapper.usize_type()?.const_int(element, false),
                "copy length",
            ),
            "copy length",
        )?;
        let bits = self.mapper.usize_type()?.get_bit_width();
        let name = if overlapping {
            format!("llvm.memmove.p0.p0.i{bits}")
        } else {
            format!("llvm.memcpy.p0.p0.i{bits}")
        };
        let function = self.void_intrinsic_function(
            &name,
            &[
                self.mapper.pointer().into(),
                self.mapper.pointer().into(),
                self.mapper.usize_type()?.into(),
                self.context.bool_type().into(),
            ],
        )?;
        self.build(
            self.builder.build_call(
                function,
                &[
                    destination.into(),
                    source.into(),
                    length.into(),
                    self.context.bool_type().const_zero().into(),
                ],
                "copy elements",
            ),
            "copy elements",
        )?;
        Ok(())
    }

    fn align_last_instruction(&self, alignment: u32, operation: &str) -> Result<(), CodegenError> {
        let instruction = self
            .builder
            .get_insert_block()
            .and_then(|block| block.get_last_instruction())
            .ok_or_else(|| CodegenError::internal(format!("{operation} emitted no instruction")))?;
        instruction
            .set_alignment(alignment)
            .map_err(|error| CodegenError::internal(format!("cannot align {operation}: {error:?}")))
    }
}
