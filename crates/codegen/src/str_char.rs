//! UTF-8 scalar decoding for `str::char_at`.
//!
//! The decoder never loads a byte before proving that the index is inside the
//! string. Each continuation byte and each scalar range is checked explicitly,
//! so an unchecked `*str` cannot turn a malformed byte sequence into undefined
//! behaviour or an out-of-bounds read.

use inkwell::IntPredicate;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};
use seal_methods::{STR_CHAR_AT_NOT_BOUNDARY, STR_CHAR_AT_OUT_OF_BOUNDS, STR_INVALID_UTF8};
use seal_span::Span;
use seal_ty::UintTy;

use crate::error::CodegenError;
use crate::function::FunctionLowerer;

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    /// Decodes the scalar that starts at `index`.
    ///
    /// The caller has already extracted the string's data pointer and length.
    /// The returned value is the Unicode scalar in the language's `char`
    /// representation, a 32-bit integer.
    pub(crate) fn str_char_at(
        &self,
        data: PointerValue<'ctx>,
        length: IntValue<'ctx>,
        index: IntValue<'ctx>,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let bytes = self.tcx.uint_ty(UintTy::U8);
        let byte_ty = self.mapper.basic(bytes)?.into_int_type();
        let i32_ty = self.context.i32_type();
        let usize_ty = self.mapper.usize_type()?;

        let in_bounds = self.build(
            self.builder
                .build_int_compare(IntPredicate::ULT, index, length, "char_at in bounds"),
            "char_at bounds comparison",
        )?;
        self.check_or_panic(in_bounds, span, STR_CHAR_AT_OUT_OF_BOUNDS)?;

        let lead = self.load_string_byte(data, index, 0, byte_ty, bytes)?;
        let continuation_mask = self.build(
            self.builder
                .build_and(lead, byte_ty.const_int(0xc0, false), "char_at lead mask"),
            "char_at lead mask",
        )?;
        let boundary = self.build(
            self.builder.build_int_compare(
                IntPredicate::NE,
                continuation_mask,
                byte_ty.const_int(0x80, false),
                "char_at boundary",
            ),
            "char_at boundary comparison",
        )?;
        self.check_or_panic(boundary, span, STR_CHAR_AT_NOT_BOUNDARY)?;

        let remaining = self.build(
            self.builder
                .build_int_sub(length, index, "char_at remaining"),
            "char_at remaining",
        )?;
        let is_ascii = self.build(
            self.builder.build_int_compare(
                IntPredicate::ULT,
                lead,
                byte_ty.const_int(0x80, false),
                "char_at ascii",
            ),
            "char_at ascii comparison",
        )?;
        let is_two = self.build(
            self.builder.build_int_compare(
                IntPredicate::EQ,
                self.build(
                    self.builder.build_and(
                        lead,
                        byte_ty.const_int(0xe0, false),
                        "char_at two mask",
                    ),
                    "char_at two mask",
                )?,
                byte_ty.const_int(0xc0, false),
                "char_at two",
            ),
            "char_at two comparison",
        )?;
        let is_three = self.build(
            self.builder.build_int_compare(
                IntPredicate::EQ,
                self.build(
                    self.builder.build_and(
                        lead,
                        byte_ty.const_int(0xf0, false),
                        "char_at three mask",
                    ),
                    "char_at three mask",
                )?,
                byte_ty.const_int(0xe0, false),
                "char_at three",
            ),
            "char_at three comparison",
        )?;
        let is_four = self.build(
            self.builder.build_int_compare(
                IntPredicate::EQ,
                self.build(
                    self.builder.build_and(
                        lead,
                        byte_ty.const_int(0xf8, false),
                        "char_at four mask",
                    ),
                    "char_at four mask",
                )?,
                byte_ty.const_int(0xf0, false),
                "char_at four",
            ),
            "char_at four comparison",
        )?;
        let valid_lead = self.build(
            self.builder.build_or(
                self.build(
                    self.builder.build_or(is_ascii, is_two, "char_at lead"),
                    "char_at lead",
                )?,
                self.build(
                    self.builder.build_or(is_three, is_four, "char_at lead"),
                    "char_at lead",
                )?,
                "char_at lead",
            ),
            "char_at lead",
        )?;
        self.check_or_panic(valid_lead, span, STR_INVALID_UTF8)?;

        let one_block = self
            .context
            .append_basic_block(self.function, "str.char.one");
        let two_block = self
            .context
            .append_basic_block(self.function, "str.char.two");
        let three_block = self
            .context
            .append_basic_block(self.function, "str.char.three");
        let four_block = self
            .context
            .append_basic_block(self.function, "str.char.four");
        let invalid_block = self
            .context
            .append_basic_block(self.function, "str.char.invalid");
        let merge_block = self
            .context
            .append_basic_block(self.function, "str.char.merge");

        let mut branches = Vec::new();
        let mut width = usize_ty.const_int(1, false);
        for (condition, value) in [(is_two, 2), (is_three, 3), (is_four, 4)] {
            width = self
                .build(
                    self.builder.build_select(
                        condition,
                        usize_ty.const_int(value, false),
                        width,
                        "char_at width",
                    ),
                    "char_at width",
                )?
                .into_int_value();
        }
        let cases = vec![
            (usize_ty.const_int(1, false), one_block),
            (usize_ty.const_int(2, false), two_block),
            (usize_ty.const_int(3, false), three_block),
            (usize_ty.const_int(4, false), four_block),
        ];
        self.build(
            self.builder.build_switch(width, invalid_block, &cases),
            "char_at width switch",
        )?;

        self.builder.position_at_end(invalid_block);
        self.panic_at(span, STR_INVALID_UTF8)?;

        self.builder.position_at_end(one_block);
        let ascii = self.build(
            self.builder.build_int_z_extend(lead, i32_ty, "char scalar"),
            "char scalar",
        )?;
        branches.push(self.finish_char_branch(merge_block, ascii)?);

        self.builder.position_at_end(two_block);
        self.require_remaining(remaining, 2, span)?;
        let second = self.load_string_byte(data, index, 1, byte_ty, bytes)?;
        self.require_continuation(second, span)?;
        let two = self.merged(
            self.shifted(
                self.masked(lead, 0x1f, "char two lead")?,
                6,
                "char two lead",
            )?,
            self.masked(second, 0x3f, "char two trail")?,
            "char two scalar",
        )?;
        self.require_codepoint(two, 0x80, None, None, span)?;
        branches.push(self.finish_char_branch(merge_block, two)?);

        self.builder.position_at_end(three_block);
        self.require_remaining(remaining, 3, span)?;
        let second = self.load_string_byte(data, index, 1, byte_ty, bytes)?;
        let third = self.load_string_byte(data, index, 2, byte_ty, bytes)?;
        self.require_continuation(second, span)?;
        self.require_continuation(third, span)?;
        let first = self.shifted(
            self.masked(lead, 0x0f, "char three lead")?,
            12,
            "char three lead",
        )?;
        let second = self.shifted(
            self.masked(second, 0x3f, "char three trail")?,
            6,
            "char three trail",
        )?;
        let third = self.masked(third, 0x3f, "char three tail")?;
        let three = self.merged(
            self.merged(first, second, "char three scalar")?,
            third,
            "char three scalar",
        )?;
        self.require_codepoint(three, 0x800, Some(0xd800), Some(0xdfff), span)?;
        branches.push(self.finish_char_branch(merge_block, three)?);

        self.builder.position_at_end(four_block);
        self.require_remaining(remaining, 4, span)?;
        let second = self.load_string_byte(data, index, 1, byte_ty, bytes)?;
        let third = self.load_string_byte(data, index, 2, byte_ty, bytes)?;
        let fourth = self.load_string_byte(data, index, 3, byte_ty, bytes)?;
        self.require_continuation(second, span)?;
        self.require_continuation(third, span)?;
        self.require_continuation(fourth, span)?;
        let first = self.shifted(
            self.masked(lead, 0x07, "char four lead")?,
            18,
            "char four lead",
        )?;
        let second = self.shifted(
            self.masked(second, 0x3f, "char four trail")?,
            12,
            "char four trail",
        )?;
        let third = self.shifted(
            self.masked(third, 0x3f, "char four trail")?,
            6,
            "char four trail",
        )?;
        let fourth = self.masked(fourth, 0x3f, "char four tail")?;
        let four = self.merged(
            self.merged(
                self.merged(first, second, "char four scalar")?,
                third,
                "char four scalar",
            )?,
            fourth,
            "char four scalar",
        )?;
        self.require_codepoint(four, 0x10000, None, Some(0x10ffff), span)?;
        branches.push(self.finish_char_branch(merge_block, four)?);

        self.builder.position_at_end(merge_block);
        let phi = self.build(
            self.builder.build_phi(i32_ty, "char_at scalar"),
            "char_at phi",
        )?;
        for (value, block) in &branches {
            phi.add_incoming(&[(value, *block)]);
        }
        Ok(phi.as_basic_value())
    }

    fn zext_byte(&self, value: IntValue<'ctx>) -> Result<IntValue<'ctx>, CodegenError> {
        self.build(
            self.builder
                .build_int_z_extend(value, self.context.i32_type(), "char byte"),
            "char byte extension",
        )
    }

    fn masked(
        &self,
        byte: IntValue<'ctx>,
        mask: u64,
        operation: &str,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        self.build(
            self.builder.build_and(
                self.zext_byte(byte)?,
                self.context.i32_type().const_int(mask, false),
                operation,
            ),
            operation,
        )
    }

    fn shifted(
        &self,
        value: IntValue<'ctx>,
        amount: u64,
        operation: &str,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        self.build(
            self.builder.build_left_shift(
                value,
                self.context.i32_type().const_int(amount, false),
                operation,
            ),
            operation,
        )
    }

    fn merged(
        &self,
        left: IntValue<'ctx>,
        right: IntValue<'ctx>,
        operation: &str,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        self.build(self.builder.build_or(left, right, operation), operation)
    }

    fn load_string_byte(
        &self,
        data: PointerValue<'ctx>,
        index: IntValue<'ctx>,
        offset: u64,
        byte_ty: inkwell::types::IntType<'ctx>,
        bytes: seal_ty::TyId,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let position = self.build(
            self.builder.build_int_add(
                index,
                index.get_type().const_int(offset, false),
                "char_at index",
            ),
            "char_at index",
        )?;
        let pointer = self.index_ptr(data, position, bytes)?;
        Ok(self
            .build(
                self.builder.build_load(byte_ty, pointer, "char_at byte"),
                "char_at byte load",
            )?
            .into_int_value())
    }

    fn require_remaining(
        &self,
        remaining: IntValue<'ctx>,
        width: u64,
        span: Span,
    ) -> Result<(), CodegenError> {
        let enough = self.build(
            self.builder.build_int_compare(
                IntPredicate::UGE,
                remaining,
                remaining.get_type().const_int(width, false),
                "char_at remaining width",
            ),
            "char_at remaining comparison",
        )?;
        self.check_or_panic(enough, span, STR_INVALID_UTF8)
    }

    fn require_continuation(&self, byte: IntValue<'ctx>, span: Span) -> Result<(), CodegenError> {
        let byte_ty = byte.get_type();
        let continuation = self.build(
            self.builder.build_int_compare(
                IntPredicate::EQ,
                self.build(
                    self.builder
                        .build_and(byte, byte_ty.const_int(0xc0, false), "char trail mask"),
                    "char trail mask",
                )?,
                byte_ty.const_int(0x80, false),
                "char trail",
            ),
            "char trail comparison",
        )?;
        self.check_or_panic(continuation, span, STR_INVALID_UTF8)
    }

    fn require_codepoint(
        &self,
        value: IntValue<'ctx>,
        minimum: u64,
        surrogate_min: Option<u64>,
        maximum: Option<u64>,
        span: Span,
    ) -> Result<(), CodegenError> {
        let i32_ty = self.context.i32_type();
        let above_minimum = self.build(
            self.builder.build_int_compare(
                IntPredicate::UGE,
                value,
                i32_ty.const_int(minimum, false),
                "char scalar minimum",
            ),
            "char scalar minimum",
        )?;
        self.check_or_panic(above_minimum, span, STR_INVALID_UTF8)?;
        if let (Some(low), Some(high)) = (surrogate_min, maximum) {
            let outside_surrogates = self.build(
                self.builder.build_or(
                    self.build(
                        self.builder.build_int_compare(
                            IntPredicate::ULT,
                            value,
                            i32_ty.const_int(low, false),
                            "char surrogate low",
                        ),
                        "char surrogate low",
                    )?,
                    self.build(
                        self.builder.build_int_compare(
                            IntPredicate::UGT,
                            value,
                            i32_ty.const_int(high, false),
                            "char surrogate high",
                        ),
                        "char surrogate high",
                    )?,
                    "char surrogate",
                ),
                "char surrogate",
            )?;
            self.check_or_panic(outside_surrogates, span, STR_INVALID_UTF8)?;
        } else if let Some(high) = maximum {
            let below_maximum = self.build(
                self.builder.build_int_compare(
                    IntPredicate::ULE,
                    value,
                    i32_ty.const_int(high, false),
                    "char scalar maximum",
                ),
                "char scalar maximum",
            )?;
            self.check_or_panic(below_maximum, span, STR_INVALID_UTF8)?;
        }
        Ok(())
    }

    fn finish_char_branch(
        &self,
        merge: inkwell::basic_block::BasicBlock<'ctx>,
        value: IntValue<'ctx>,
    ) -> Result<(IntValue<'ctx>, inkwell::basic_block::BasicBlock<'ctx>), CodegenError> {
        let block = self.builder.get_insert_block().ok_or_else(|| {
            CodegenError::internal("a char decoder branch has no insertion block")
        })?;
        self.build(
            self.builder.build_unconditional_branch(merge),
            "char branch",
        )?;
        Ok((value, block))
    }
}
