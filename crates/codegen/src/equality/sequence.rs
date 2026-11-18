//! Byte and element sequence comparison.
//!
//! A sequence is compared by length first and then by elements. The element
//! loop always runs over the shorter length, so a length mismatch never reads
//! past either value and needs no early exit.

use inkwell::IntPredicate;
use inkwell::values::{IntValue, PointerValue};
use seal_span::Span;
use seal_ty::{TyId, UintTy};

use crate::error::CodegenError;
use crate::function::FunctionLowerer;

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    /// Compares two `*str` values by length and bytes.
    pub(super) fn equal_str(
        &self,
        ty: TyId,
        lhs: PointerValue<'ctx>,
        rhs: PointerValue<'ctx>,
        span: Span,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let fat_type = self.mapper.basic(ty)?.into_struct_type();
        let (lhs_data, lhs_len) = self.fat_parts(fat_type, lhs, "lhs")?;
        let (rhs_data, rhs_len) = self.fat_parts(fat_type, rhs, "rhs")?;
        let lengths_equal = self.equal_lengths(lhs_len, rhs_len)?;
        let bytes = self.tcx.uint_ty(UintTy::U8);
        let common = self.common_length(lhs_len, rhs_len)?;
        let bytes_equal = self.equal_elements(bytes, lhs_data, rhs_data, common, span)?;
        self.build(
            self.builder
                .build_and(lengths_equal, bytes_equal, "equality str"),
            "equality str",
        )
    }

    /// Compares two `*[T]` values by length and elements.
    pub(super) fn equal_slice(
        &self,
        ty: TyId,
        element: TyId,
        lhs: PointerValue<'ctx>,
        rhs: PointerValue<'ctx>,
        span: Span,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let fat_type = self.mapper.basic(ty)?.into_struct_type();
        let (lhs_data, lhs_len) = self.fat_parts(fat_type, lhs, "lhs")?;
        let (rhs_data, rhs_len) = self.fat_parts(fat_type, rhs, "rhs")?;
        let lengths_equal = self.equal_lengths(lhs_len, rhs_len)?;
        let common = self.common_length(lhs_len, rhs_len)?;
        let elements_equal = self.equal_elements(element, lhs_data, rhs_data, common, span)?;
        self.build(
            self.builder
                .build_and(lengths_equal, elements_equal, "equality slice"),
            "equality slice",
        )
    }

    /// Compares `count` consecutive `element` values at `lhs` and `rhs`.
    ///
    /// The loop accumulates `false` as soon as one element differs instead of
    /// branching out, which keeps a single exit for the caller.
    pub(super) fn equal_elements(
        &self,
        element: TyId,
        lhs: PointerValue<'ctx>,
        rhs: PointerValue<'ctx>,
        count: IntValue<'ctx>,
        span: Span,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let usize_type = self.mapper.usize_type()?;
        let bool_type = self.context.bool_type();
        let element_size = self.mapper.layout(element)?.size();
        let index_slot = self.entry_alloca(usize_type.into(), "equality.index")?;
        let equal_slot = self.entry_alloca(bool_type.into(), "equality.elements")?;
        self.build(
            self.builder
                .build_store(index_slot, usize_type.const_zero()),
            "equality index init",
        )?;
        self.build(
            self.builder
                .build_store(equal_slot, bool_type.const_int(1, false)),
            "equality elements init",
        )?;

        let loop_block = self
            .context
            .append_basic_block(self.function, "equality.loop");
        let body_block = self
            .context
            .append_basic_block(self.function, "equality.body");
        let done_block = self
            .context
            .append_basic_block(self.function, "equality.elements.done");
        self.build(
            self.builder.build_unconditional_branch(loop_block),
            "equality loop entry",
        )?;

        self.builder.position_at_end(loop_block);
        let index = self
            .build(
                self.builder
                    .build_load(usize_type, index_slot, "equality index"),
                "equality index load",
            )?
            .into_int_value();
        let within = self.build(
            self.builder.build_int_compare(
                IntPredicate::ULT,
                index,
                count,
                "equality within bounds",
            ),
            "equality bounds",
        )?;
        self.build(
            self.builder
                .build_conditional_branch(within, body_block, done_block),
            "equality loop branch",
        )?;

        self.builder.position_at_end(body_block);
        let offset = self.build(
            self.builder.build_int_mul(
                index,
                usize_type.const_int(element_size, false),
                "equality offset",
            ),
            "equality element offset",
        )?;
        let lhs_element = self.offset_by(lhs, offset)?;
        let rhs_element = self.offset_by(rhs, offset)?;
        let same = self.equal_memory(element, lhs_element, rhs_element, span)?;
        let accumulated = self
            .build(
                self.builder
                    .build_load(bool_type, equal_slot, "equality accumulated"),
                "equality accumulated load",
            )?
            .into_int_value();
        let combined = self.build(
            self.builder
                .build_and(accumulated, same, "equality element"),
            "equality element",
        )?;
        self.build(
            self.builder.build_store(equal_slot, combined),
            "equality accumulated store",
        )?;
        let next = self.build(
            self.builder
                .build_int_add(index, usize_type.const_int(1, false), "equality next"),
            "equality index increment",
        )?;
        self.build(
            self.builder.build_store(index_slot, next),
            "equality index store",
        )?;
        self.build(
            self.builder.build_unconditional_branch(loop_block),
            "equality back edge",
        )?;

        self.builder.position_at_end(done_block);
        Ok(self
            .build(
                self.builder
                    .build_load(bool_type, equal_slot, "equality elements"),
                "equality elements load",
            )?
            .into_int_value())
    }

    /// Extracts the data pointer and length of a fat pointer stored at `slot`.
    fn fat_parts(
        &self,
        fat_type: inkwell::types::StructType<'ctx>,
        slot: PointerValue<'ctx>,
        side: &str,
    ) -> Result<(PointerValue<'ctx>, IntValue<'ctx>), CodegenError> {
        let fat = self
            .build(
                self.builder
                    .build_load(fat_type, slot, &format!("equality {side} fat pointer")),
                "equality fat pointer load",
            )?
            .into_struct_value();
        let data = self
            .build(
                self.builder
                    .build_extract_value(fat, 0, &format!("equality {side} data")),
                "equality data",
            )?
            .into_pointer_value();
        let length = self
            .build(
                self.builder
                    .build_extract_value(fat, 1, &format!("equality {side} length")),
                "equality length",
            )?
            .into_int_value();
        Ok((data, length))
    }

    /// Returns `lhs == rhs` for two lengths.
    fn equal_lengths(
        &self,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        self.build(
            self.builder
                .build_int_compare(IntPredicate::EQ, lhs, rhs, "equality lengths"),
            "equality length comparison",
        )
    }

    /// Returns the smaller of two lengths, so a comparison never reads past
    /// either value.
    fn common_length(
        &self,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let shorter = self.build(
            self.builder
                .build_int_compare(IntPredicate::ULT, lhs, rhs, "equality shorter"),
            "equality length order",
        )?;
        Ok(self
            .build(
                self.builder
                    .build_select(shorter, lhs, rhs, "equality common length"),
                "equality common length",
            )?
            .into_int_value())
    }
}
