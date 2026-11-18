//! Field and variant comparison.
//!
//! Fields are addressed by the offsets the layout assigns them, and an enum is
//! compared through its tag so only the active variant's fields are read.

use inkwell::IntPredicate;
use inkwell::values::{IntValue, PointerValue};
use seal_span::Span;
use seal_ty::{AdtId, SubstsId, TyId};

use crate::error::CodegenError;
use crate::function::FunctionLowerer;

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    /// Compares every field at its layout offset.
    ///
    /// A zero-sized field carries no state, so it is skipped: its offset may
    /// coincide with the next field's, and comparing it would read a value that
    /// is not there.
    pub(super) fn equal_fields(
        &self,
        fields: &[(TyId, u64)],
        lhs: PointerValue<'ctx>,
        rhs: PointerValue<'ctx>,
        span: Span,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let usize_type = self.mapper.usize_type()?;
        let mut equal = self.true_value();
        for (index, (field, offset)) in fields.iter().enumerate() {
            if self.mapper.layout(*field)?.size() == 0 {
                continue;
            }
            let offset = usize_type.const_int(*offset, false);
            let lhs_field = self.offset_by(lhs, offset)?;
            let rhs_field = self.offset_by(rhs, offset)?;
            let same = self.equal_memory(*field, lhs_field, rhs_field, span)?;
            equal = self.build(
                self.builder
                    .build_and(equal, same, &format!("equality.field{index}")),
                "equality field",
            )?;
        }
        Ok(equal)
    }

    /// Compares two enums by tag, then by the active variant's fields.
    pub(super) fn equal_enum(
        &self,
        adt: AdtId,
        args: SubstsId,
        lhs: PointerValue<'ctx>,
        rhs: PointerValue<'ctx>,
        span: Span,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let layout = self.mapper.enum_layout(adt, args)?;
        let tag_bits = u16::try_from(layout.tag.size() * 8)
            .map_err(|_| CodegenError::internal("an enum tag is too wide for LLVM"))?;
        let tag_type = self.mapper.integer_type(tag_bits)?;
        let lhs_tag = self
            .build(
                self.builder.build_load(tag_type, lhs, "equality lhs tag"),
                "equality tag load",
            )?
            .into_int_value();
        let rhs_tag = self
            .build(
                self.builder.build_load(tag_type, rhs, "equality rhs tag"),
                "equality tag load",
            )?
            .into_int_value();
        let same_tag = self.build(
            self.builder
                .build_int_compare(IntPredicate::EQ, lhs_tag, rhs_tag, "equality tags"),
            "equality tag comparison",
        )?;

        let bool_type = self.context.bool_type();
        let result = self.entry_alloca(bool_type.into(), "equality.result")?;
        self.build(
            self.builder.build_store(result, bool_type.const_zero()),
            "equality result init",
        )?;

        let payload_block = self
            .context
            .append_basic_block(self.function, "equality.payload");
        let done_block = self
            .context
            .append_basic_block(self.function, "equality.done");
        self.build(
            self.builder
                .build_conditional_branch(same_tag, payload_block, done_block),
            "equality tag branch",
        )?;

        self.builder.position_at_end(payload_block);
        let definition = self.tcx.adt(adt);
        let variant_blocks: Vec<_> = definition
            .variants
            .iter()
            .enumerate()
            .map(|(index, _)| {
                self.context
                    .append_basic_block(self.function, &format!("equality.variant{index}"))
            })
            .collect();
        let cases: Vec<(IntValue<'ctx>, _)> = variant_blocks
            .iter()
            .enumerate()
            .map(|(index, block)| (tag_type.const_int(index as u64, false), *block))
            .collect();
        self.build(
            self.builder.build_switch(lhs_tag, done_block, &cases),
            "equality variant switch",
        )?;

        for (index, (block, variant)) in variant_blocks.iter().zip(&definition.variants).enumerate()
        {
            self.builder.position_at_end(*block);
            let variant_layout = layout.variants.get(index).ok_or_else(|| {
                CodegenError::internal(format!("enum {adt:?} has no layout for variant {index}"))
            })?;
            let fields: Vec<(TyId, u64)> = variant
                .fields
                .iter()
                .zip(&variant_layout.fields)
                .map(|(field, field_layout)| {
                    (self.tcx.substitute(field.ty, args), field_layout.offset)
                })
                .collect();
            let equal = self.equal_fields(&fields, lhs, rhs, span)?;
            self.build(
                self.builder.build_store(result, equal),
                "equality variant result",
            )?;
            self.build(
                self.builder.build_unconditional_branch(done_block),
                "equality variant branch",
            )?;
        }

        self.builder.position_at_end(done_block);
        Ok(self
            .build(
                self.builder
                    .build_load(bool_type, result, "equality result"),
                "equality result load",
            )?
            .into_int_value())
    }
}
