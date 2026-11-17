//! Evaluating right-hand sides.
//!
//! Integer division and remainder panic through the runtime instead of invoking
//! LLVM's undefined behaviour for a zero divisor or for the signed minimum
//! divided by minus one. Shifts mask the amount to the operand width, so an
//! oversized shift is defined and deterministic rather than target-dependent.

use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FloatValue, IntValue, ValueKind};
use inkwell::{FloatPredicate, IntPredicate};
use seal_def::DefId;
use seal_hash::FxHashMap;
use seal_mir::{AggregateKind, BinOp, Rvalue, UnOp};
use seal_span::Span;
use seal_ty::{Instance, SubstsId, TyKind, VariantIdx};

use crate::error::CodegenError;
use crate::function::FunctionLowerer;

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    /// Evaluates `rvalue` and returns its value.
    pub(crate) fn emit_rvalue(
        &self,
        rvalue: &Rvalue,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match rvalue {
            Rvalue::Use(operand) => self.load_operand(operand),
            Rvalue::Discriminant(place) => self.discriminant(place),
            Rvalue::Ref { place, .. } => Ok(self.place_ptr(place)?.into()),
            Rvalue::SliceRef {
                base,
                start,
                end,
                ty,
                ..
            } => self.slice_ref(base, start.as_ref(), end.as_ref(), *ty),
            Rvalue::Len { slice } => self.slice_len(slice),
            Rvalue::Binary { op, lhs, rhs, span } => {
                let ty = self.operand_type(lhs)?;
                let lhs = self.load_operand(lhs)?;
                let rhs = self.load_operand(rhs)?;
                self.binary(*op, lhs, rhs, ty, *span)
            }
            Rvalue::Unary { op, operand } => {
                let value = self.load_operand(operand)?;
                self.unary(*op, value)
            }
            Rvalue::Intrinsic {
                method,
                receiver,
                args,
                span,
            } => self.emit_intrinsic(*method, receiver, args, *span),
            Rvalue::Call { target, args, .. } => {
                if target.is_trait_method() {
                    return Err(CodegenError::internal(
                        "a symbolic trait call reached code generation; monomorphization must \
                         resolve it first",
                    ));
                }
                self.call(target.instance(), args)
            }
            Rvalue::Aggregate { kind, fields } => self.aggregate(*kind, fields),
            Rvalue::Tuple { elements, ty } => self.tuple_aggregate(elements, *ty),
            Rvalue::Array { elements, ty } => self.array_aggregate(elements, *ty),
            Rvalue::Repeat { value, count, ty } => self.array_repeat(value, *count, *ty),
        }
    }

    fn discriminant(&self, place: &seal_mir::Place) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let ty = self.place_type(place)?;
        let TyKind::Adt { adt, args } = self.tcx.kind(ty) else {
            return Err(CodegenError::internal(format!(
                "cannot read a discriminant from `{ty:?}`"
            )));
        };
        let enum_type = self.mapper.adt(adt, args)?;
        let pointer = self.place_ptr(place)?;
        let layout = self.mapper.enum_layout(adt, args)?;
        let tag_bits = u16::try_from(layout.tag.size() * 8).map_err(|_| {
            CodegenError::internal(format!("enum {adt:?} has a tag too wide for codegen"))
        })?;
        let tag_type = self.mapper.integer_type(tag_bits)?;
        let tag_pointer = self.build(
            self.builder
                .build_struct_gep(enum_type, pointer, 0, "enum.tag"),
            "enum tag projection",
        )?;
        let tag = self
            .build(
                self.builder.build_load(tag_type, tag_pointer, "enum tag"),
                "enum tag load",
            )?
            .into_int_value();
        let wide = self.context.i32_type();
        Ok(self
            .build(
                self.builder.build_int_z_extend(tag, wide, "discriminant"),
                "discriminant extension",
            )?
            .into())
    }

    fn slice_ref(
        &self,
        base: &seal_mir::Place,
        start: Option<&seal_mir::Operand>,
        end: Option<&seal_mir::Operand>,
        ty: seal_ty::TyId,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let base_ty = self.place_type(base)?;
        let (element, data, length) = match self.tcx.kind(base_ty) {
            TyKind::Array { element, length } => (
                element,
                self.place_ptr(base)?,
                self.mapper.usize_type()?.const_int(length, false),
            ),
            TyKind::Ref { pointee, .. } => match self.tcx.kind(pointee) {
                TyKind::Array { element, length } => {
                    let pointer = self
                        .load_operand(&seal_mir::Operand::Copy(base.clone()))?
                        .into_pointer_value();
                    (
                        element,
                        pointer,
                        self.mapper.usize_type()?.const_int(length, false),
                    )
                }
                TyKind::Slice { element } => {
                    let fat = self
                        .load_operand(&seal_mir::Operand::Copy(base.clone()))?
                        .into_struct_value();
                    let data = self
                        .build(
                            self.builder.build_extract_value(fat, 0, "slice data"),
                            "slice data",
                        )?
                        .into_pointer_value();
                    let length = self
                        .build(
                            self.builder.build_extract_value(fat, 1, "slice length"),
                            "slice length",
                        )?
                        .into_int_value();
                    (element, data, length)
                }
                _ => {
                    return Err(CodegenError::internal(format!(
                        "cannot slice `{base_ty:?}` during code generation"
                    )));
                }
            },
            _ => {
                return Err(CodegenError::internal(format!(
                    "cannot slice `{base_ty:?}` during code generation"
                )));
            }
        };

        let zero = self.mapper.usize_type()?.const_zero();
        let start = match start {
            Some(start) => self.load_operand(start)?.into_int_value(),
            None => zero,
        };
        let end = match end {
            Some(end) => self.load_operand(end)?.into_int_value(),
            None => length,
        };
        let data = self.index_ptr(data, start, element)?;
        let length = self.build(
            self.builder.build_int_sub(end, start, "slice length"),
            "slice length",
        )?;

        let fat_type = self.mapper.basic(ty)?.into_struct_type();
        let mut fat = fat_type.get_undef();
        let data: BasicValueEnum<'ctx> = data.into();
        let next = self.build(
            self.builder.build_insert_value(fat, data, 0, "slice data"),
            "slice data",
        )?;
        fat = match next {
            inkwell::values::AggregateValueEnum::StructValue(value) => value,
            other => {
                return Err(CodegenError::internal(format!(
                    "slice construction produced {other:?}"
                )));
            }
        };
        let length: BasicValueEnum<'ctx> = length.into();
        let next = self.build(
            self.builder
                .build_insert_value(fat, length, 1, "slice length"),
            "slice length",
        )?;
        match next {
            inkwell::values::AggregateValueEnum::StructValue(value) => Ok(value.into()),
            other => Err(CodegenError::internal(format!(
                "slice construction produced {other:?}"
            ))),
        }
    }

    fn slice_len(&self, slice: &seal_mir::Operand) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let ty = self.operand_type(slice)?;
        let fat = self.load_operand(slice)?.into_struct_value();
        let _ = self.mapper.basic(ty)?;
        self.build(
            self.builder.build_extract_value(fat, 1, "slice length"),
            "slice length",
        )
    }

    fn tuple_aggregate(
        &self,
        elements: &[seal_mir::Operand],
        ty: seal_ty::TyId,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        if matches!(self.tcx.kind(ty), TyKind::Unit) {
            if !elements.is_empty() {
                return Err(CodegenError::internal(
                    "a non-empty tuple literal cannot have the unit type".to_owned(),
                ));
            }
            return Ok(self.mapper.basic(ty)?.into_struct_type().get_undef().into());
        }
        let TyKind::Tuple(tuple) = self.tcx.kind(ty) else {
            return Err(CodegenError::internal(format!(
                "tuple construction has non-tuple type `{ty:?}`"
            )));
        };
        let element_types = self.tcx.tuple_elements(tuple);
        if elements.len() != element_types.len() {
            return Err(CodegenError::internal(format!(
                "tuple construction has {} elements for an arity of {}",
                elements.len(),
                element_types.len()
            )));
        }
        let tuple_type = self.mapper.basic(ty)?.into_struct_type();
        let mut aggregate = tuple_type.get_undef();
        for (index, element) in elements.iter().enumerate() {
            let value = self.load_operand(element)?;
            let next = self.build(
                self.builder
                    .build_insert_value(aggregate, value, index as u32, "tuple element"),
                "tuple element",
            )?;
            aggregate = match next {
                inkwell::values::AggregateValueEnum::StructValue(value) => value,
                other => {
                    return Err(CodegenError::internal(format!(
                        "tuple construction produced {other:?}"
                    )));
                }
            };
        }
        Ok(aggregate.into())
    }

    fn array_aggregate(
        &self,
        elements: &[seal_mir::Operand],
        ty: seal_ty::TyId,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let (_, length) = match self.tcx.kind(ty) {
            TyKind::Array { element, length } => (element, length),
            other => {
                return Err(CodegenError::internal(format!(
                    "array construction has non-array type `{other:?}`"
                )));
            }
        };
        if elements.len() as u64 != length {
            return Err(CodegenError::internal(format!(
                "array construction has {} elements for a length of {length}",
                elements.len()
            )));
        }
        let array_type = self.mapper.basic(ty)?.into_array_type();
        let mut aggregate = array_type.const_zero();
        for (index, element) in elements.iter().enumerate() {
            let value = self.load_operand(element)?;
            let next = self.build(
                self.builder
                    .build_insert_value(aggregate, value, index as u32, "array element"),
                "array element",
            )?;
            aggregate = match next {
                inkwell::values::AggregateValueEnum::ArrayValue(value) => value,
                other => {
                    return Err(CodegenError::internal(format!(
                        "array construction produced {other:?}"
                    )));
                }
            };
        }
        Ok(aggregate.into())
    }

    fn array_repeat(
        &self,
        value: &seal_mir::Operand,
        count: u64,
        ty: seal_ty::TyId,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let element_ty = match self.tcx.kind(ty) {
            TyKind::Array { element, length } if length == count => element,
            TyKind::Array { length, .. } => {
                return Err(CodegenError::internal(format!(
                    "array repeat has count {count} for a length of {length}"
                )));
            }
            other => {
                return Err(CodegenError::internal(format!(
                    "array repeat has non-array type `{other:?}`"
                )));
            }
        };
        let array_type = self.mapper.basic(ty)?.into_array_type();
        if count == 0 {
            return Ok(array_type.const_zero().into());
        }

        let value = self.load_operand(value)?;
        let usize_type = self.mapper.usize_type()?;
        let storage = self.entry_alloca(array_type.into(), "array.repeat")?;
        let index_storage = self.entry_alloca(usize_type.into(), "array.index")?;
        self.build(
            self.builder
                .build_store(index_storage, usize_type.const_zero()),
            "repeat index init",
        )?;

        let loop_block = self
            .context
            .append_basic_block(self.function, "array.repeat.loop");
        let body_block = self
            .context
            .append_basic_block(self.function, "array.repeat.body");
        let done_block = self
            .context
            .append_basic_block(self.function, "array.repeat.done");
        self.build(
            self.builder.build_unconditional_branch(loop_block),
            "repeat branch",
        )?;

        self.builder.position_at_end(loop_block);
        let index = self
            .build(
                self.builder
                    .build_load(usize_type, index_storage, "array index"),
                "repeat index load",
            )?
            .into_int_value();
        let condition = self.build(
            self.builder.build_int_compare(
                IntPredicate::ULT,
                index,
                usize_type.const_int(count, false),
                "array index in range",
            ),
            "repeat comparison",
        )?;
        self.build(
            self.builder
                .build_conditional_branch(condition, body_block, done_block),
            "repeat branch",
        )?;

        self.builder.position_at_end(body_block);
        let element = self.index_ptr(storage, index, element_ty)?;
        self.build(
            self.builder.build_store(element, value),
            "repeat element store",
        )?;
        let next = self.build(
            self.builder
                .build_int_add(index, usize_type.const_int(1, false), "next"),
            "repeat index increment",
        )?;
        self.build(
            self.builder.build_store(index_storage, next),
            "repeat index store",
        )?;
        self.build(
            self.builder.build_unconditional_branch(loop_block),
            "repeat back edge",
        )?;

        self.builder.position_at_end(done_block);
        self.build(
            self.builder.build_load(array_type, storage, "array result"),
            "repeat result",
        )
    }

    fn binary(
        &self,
        op: BinOp,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
        ty: seal_ty::TyId,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match (lhs, rhs) {
            (BasicValueEnum::IntValue(lhs), BasicValueEnum::IntValue(rhs)) => {
                if lhs.get_type().get_bit_width() != rhs.get_type().get_bit_width() {
                    return Err(CodegenError::internal(
                        "integer operands of a binary operation have different widths",
                    ));
                }
                self.integer_binary(op, lhs, rhs, ty, span)
            }
            (BasicValueEnum::FloatValue(lhs), BasicValueEnum::FloatValue(rhs)) => {
                self.float_binary(op, lhs, rhs)
            }
            _ => Err(CodegenError::internal(
                "binary operands do not have the same scalar kind",
            )),
        }
    }

    fn integer_binary(
        &self,
        op: BinOp,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
        ty: seal_ty::TyId,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let signed = matches!(self.tcx.kind(ty), TyKind::Int(_));
        let value = match op {
            BinOp::Add => self.build(self.builder.build_int_add(lhs, rhs, "add"), "addition")?,
            BinOp::Sub => self.build(
                self.builder.build_int_sub(lhs, rhs, "subtract"),
                "subtraction",
            )?,
            BinOp::Mul => self.build(
                self.builder.build_int_mul(lhs, rhs, "multiply"),
                "multiplication",
            )?,
            BinOp::Div => {
                self.guard_divisor(lhs, rhs, signed, op, span)?;
                if signed {
                    self.build(
                        self.builder.build_int_signed_div(lhs, rhs, "divide"),
                        "signed division",
                    )?
                } else {
                    self.build(
                        self.builder.build_int_unsigned_div(lhs, rhs, "divide"),
                        "unsigned division",
                    )?
                }
            }
            BinOp::Rem => {
                self.guard_divisor(lhs, rhs, signed, op, span)?;
                if signed {
                    self.build(
                        self.builder.build_int_signed_rem(lhs, rhs, "remainder"),
                        "signed remainder",
                    )?
                } else {
                    self.build(
                        self.builder.build_int_unsigned_rem(lhs, rhs, "remainder"),
                        "unsigned remainder",
                    )?
                }
            }
            BinOp::Eq => self.build(
                self.builder
                    .build_int_compare(IntPredicate::EQ, lhs, rhs, "equal"),
                "comparison",
            )?,
            BinOp::Ne => self.build(
                self.builder
                    .build_int_compare(IntPredicate::NE, lhs, rhs, "not_equal"),
                "comparison",
            )?,
            BinOp::Lt => {
                let predicate = if signed {
                    IntPredicate::SLT
                } else {
                    IntPredicate::ULT
                };
                self.build(
                    self.builder.build_int_compare(predicate, lhs, rhs, "less"),
                    "comparison",
                )?
            }
            BinOp::Gt => {
                let predicate = if signed {
                    IntPredicate::SGT
                } else {
                    IntPredicate::UGT
                };
                self.build(
                    self.builder
                        .build_int_compare(predicate, lhs, rhs, "greater"),
                    "comparison",
                )?
            }
            BinOp::Le => {
                let predicate = if signed {
                    IntPredicate::SLE
                } else {
                    IntPredicate::ULE
                };
                self.build(
                    self.builder
                        .build_int_compare(predicate, lhs, rhs, "less_equal"),
                    "comparison",
                )?
            }
            BinOp::Ge => {
                let predicate = if signed {
                    IntPredicate::SGE
                } else {
                    IntPredicate::UGE
                };
                self.build(
                    self.builder
                        .build_int_compare(predicate, lhs, rhs, "greater_equal"),
                    "comparison",
                )?
            }
            BinOp::BitAnd => self.build(self.builder.build_and(lhs, rhs, "and"), "bitwise and")?,
            BinOp::BitOr => self.build(self.builder.build_or(lhs, rhs, "or"), "bitwise or")?,
            BinOp::BitXor => self.build(self.builder.build_xor(lhs, rhs, "xor"), "bitwise xor")?,
            BinOp::Shl => {
                let amount = self.shift_amount(rhs)?;
                self.build(
                    self.builder.build_left_shift(lhs, amount, "shift_left"),
                    "left shift",
                )?
            }
            BinOp::Shr => {
                let amount = self.shift_amount(rhs)?;
                self.build(
                    self.builder
                        .build_right_shift(lhs, amount, signed, "shift_right"),
                    "right shift",
                )?
            }
        };
        Ok(value.into())
    }

    fn float_binary(
        &self,
        op: BinOp,
        lhs: FloatValue<'ctx>,
        rhs: FloatValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        Ok(match op {
            BinOp::Add => self
                .build(
                    self.builder.build_float_add(lhs, rhs, "add"),
                    "float addition",
                )?
                .into(),
            BinOp::Sub => self
                .build(
                    self.builder.build_float_sub(lhs, rhs, "subtract"),
                    "float subtraction",
                )?
                .into(),
            BinOp::Mul => self
                .build(
                    self.builder.build_float_mul(lhs, rhs, "multiply"),
                    "float multiplication",
                )?
                .into(),
            BinOp::Div => self
                .build(
                    self.builder.build_float_div(lhs, rhs, "divide"),
                    "float division",
                )?
                .into(),
            BinOp::Rem => self
                .build(
                    self.builder.build_float_rem(lhs, rhs, "remainder"),
                    "float remainder",
                )?
                .into(),
            BinOp::Eq => self
                .build(
                    self.builder
                        .build_float_compare(FloatPredicate::OEQ, lhs, rhs, "equal"),
                    "float comparison",
                )?
                .into(),
            BinOp::Ne => self
                .build(
                    self.builder
                        .build_float_compare(FloatPredicate::UNE, lhs, rhs, "not_equal"),
                    "float comparison",
                )?
                .into(),
            BinOp::Lt => self
                .build(
                    self.builder
                        .build_float_compare(FloatPredicate::OLT, lhs, rhs, "less"),
                    "float comparison",
                )?
                .into(),
            BinOp::Gt => self
                .build(
                    self.builder
                        .build_float_compare(FloatPredicate::OGT, lhs, rhs, "greater"),
                    "float comparison",
                )?
                .into(),
            BinOp::Le => self
                .build(
                    self.builder
                        .build_float_compare(FloatPredicate::OLE, lhs, rhs, "less_equal"),
                    "float comparison",
                )?
                .into(),
            BinOp::Ge => self
                .build(
                    self.builder.build_float_compare(
                        FloatPredicate::OGE,
                        lhs,
                        rhs,
                        "greater_equal",
                    ),
                    "float comparison",
                )?
                .into(),
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr => {
                return Err(CodegenError::internal(
                    "a bitwise operation reached a floating-point operand",
                ));
            }
        })
    }

    fn unary(
        &self,
        op: UnOp,
        value: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match (op, value) {
            (UnOp::Neg, BasicValueEnum::IntValue(value)) => Ok(self
                .build(self.builder.build_int_neg(value, "negate"), "negation")?
                .into()),
            (UnOp::Neg, BasicValueEnum::FloatValue(value)) => Ok(self
                .build(
                    self.builder.build_float_neg(value, "negate"),
                    "float negation",
                )?
                .into()),
            (UnOp::Not | UnOp::BitNot, BasicValueEnum::IntValue(value)) => Ok(self
                .build(self.builder.build_not(value, "complement"), "complement")?
                .into()),
            _ => Err(CodegenError::internal(
                "unary operand has an unexpected scalar kind",
            )),
        }
    }

    fn shift_amount(&self, rhs: IntValue<'ctx>) -> Result<IntValue<'ctx>, CodegenError> {
        let ty = rhs.get_type();
        let width = ty.get_bit_width();
        let mask = ty.const_int(u64::from(width.saturating_sub(1)), false);
        self.build(
            self.builder.build_and(rhs, mask, "shift_mask"),
            "shift mask",
        )
    }

    fn guard_divisor(
        &self,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
        signed: bool,
        op: BinOp,
        span: Span,
    ) -> Result<(), CodegenError> {
        let (zero_message, overflow_message) = match op {
            BinOp::Div => (
                "attempt to divide by zero",
                "attempt to divide with overflow",
            ),
            BinOp::Rem => (
                "attempt to calculate the remainder with a divisor of zero",
                "attempt to calculate the remainder with overflow",
            ),
            _ => {
                return Err(CodegenError::internal(
                    "a non-division operation reached the division guard",
                ));
            }
        };

        let ty = rhs.get_type();
        let zero = ty.const_zero();
        let is_zero = self.build(
            self.builder
                .build_int_compare(IntPredicate::EQ, rhs, zero, "divisor_is_zero"),
            "divisor check",
        )?;
        let overflow = if signed {
            let width = ty.get_bit_width();
            let minus_one = if width > 64 {
                ty.const_int_arbitrary_precision(&[u64::MAX, u64::MAX])
            } else {
                ty.const_int(u64::MAX, false)
            };
            let is_minus_one = self.build(
                self.builder.build_int_compare(
                    IntPredicate::EQ,
                    rhs,
                    minus_one,
                    "divisor_is_minus_one",
                ),
                "divisor check",
            )?;
            let minimum = self.int_constant(ty, 1u128 << (width - 1));
            let is_minimum = self.build(
                self.builder.build_int_compare(
                    IntPredicate::EQ,
                    lhs,
                    minimum,
                    "dividend_is_minimum",
                ),
                "dividend check",
            )?;
            Some(
                self.build(
                    self.builder
                        .build_and(is_minus_one, is_minimum, "division_overflows"),
                    "overflow check",
                )?,
            )
        } else {
            None
        };

        let zero_block = self
            .context
            .append_basic_block(self.function, "division.zero");
        let continuation = self
            .context
            .append_basic_block(self.function, "division.continue");
        let overflow_block = overflow.map(|_| {
            self.context
                .append_basic_block(self.function, "division.overflow")
        });

        match (overflow, overflow_block) {
            (Some(overflow), Some(overflow_block)) => {
                let check_block = self
                    .context
                    .append_basic_block(self.function, "division.check_overflow");
                self.build(
                    self.builder
                        .build_conditional_branch(is_zero, zero_block, check_block),
                    "division branch",
                )?;
                self.builder.position_at_end(check_block);
                self.build(
                    self.builder
                        .build_conditional_branch(overflow, overflow_block, continuation),
                    "overflow branch",
                )?;
                self.builder.position_at_end(zero_block);
                self.panic_at(span, zero_message)?;
                self.builder.position_at_end(overflow_block);
                self.panic_at(span, overflow_message)?;
            }
            _ => {
                self.build(
                    self.builder
                        .build_conditional_branch(is_zero, zero_block, continuation),
                    "division branch",
                )?;
                self.builder.position_at_end(zero_block);
                self.panic_at(span, zero_message)?;
            }
        }
        self.builder.position_at_end(continuation);
        Ok(())
    }

    fn call(
        &self,
        instance: Instance,
        arguments: &[seal_mir::Operand],
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let function = self.functions.get(&instance).copied().ok_or_else(|| {
            CodegenError::internal(format!("call target {instance:?} was not declared"))
        })?;
        let mut values = Vec::with_capacity(arguments.len());
        for argument in arguments {
            values.push(BasicMetadataValueEnum::from(self.load_operand(argument)?));
        }
        let call = self.build(
            self.builder.build_call(function, &values, "call"),
            "function call",
        )?;
        match call.try_as_basic_value() {
            ValueKind::Basic(value) => Ok(value),
            ValueKind::Instruction(_) => Err(CodegenError::internal(
                "a Seal function returned no LLVM value",
            )),
        }
    }

    fn aggregate(
        &self,
        kind: AggregateKind,
        fields: &[(DefId, seal_mir::Operand)],
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match kind {
            AggregateKind::Struct { adt, args } => self.struct_aggregate(adt, args, fields),
            AggregateKind::Variant { adt, variant, args } => {
                self.variant_aggregate(adt, variant, args, fields)
            }
        }
    }

    fn struct_aggregate(
        &self,
        def: DefId,
        args: SubstsId,
        fields: &[(DefId, seal_mir::Operand)],
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let adt = self.adt_of(def)?;
        let definition = self.tcx.adt(adt);
        let struct_type = self.mapper.adt(adt, args)?;

        // Field operands are in evaluation order. Reads have no side effects, so
        // collecting them first and inserting in declaration order preserves the
        // meaning while producing a well-formed LLVM struct.
        let mut values = FxHashMap::default();
        for (field, operand) in fields {
            values.insert(*field, self.load_operand(operand)?);
        }
        let mut aggregate = struct_type.get_undef();
        for (index, field) in definition.fields.iter().enumerate() {
            let value = values.remove(&field.def).ok_or_else(|| {
                CodegenError::internal(format!(
                    "aggregate {adt:?} is missing an initializer for {field:?}"
                ))
            })?;
            let next = self.build(
                self.builder
                    .build_insert_value(aggregate, value, index as u32, "field"),
                "aggregate field",
            )?;
            aggregate = match next {
                inkwell::values::AggregateValueEnum::StructValue(value) => value,
                other => {
                    return Err(CodegenError::internal(format!(
                        "aggregate construction produced {other:?}"
                    )));
                }
            };
        }
        Ok(aggregate.into())
    }

    /// Builds one enum variant by writing its discriminant and payload fields.
    ///
    /// The storage is a temporary in the function prologue, so constructing an
    /// enum inside a loop does not grow the stack. The payload is addressed as
    /// bytes: every variant shares one LLVM type, and the language layout gives
    /// each field's absolute offset.
    fn variant_aggregate(
        &self,
        def: DefId,
        variant: VariantIdx,
        args: SubstsId,
        fields: &[(DefId, seal_mir::Operand)],
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let adt = self.adt_of(def)?;
        let layout = self.mapper.enum_layout(adt, args)?;
        let definition = self.tcx.adt(adt);
        let variant_def = definition.variant(variant).ok_or_else(|| {
            CodegenError::internal(format!(
                "variant {variant:?} does not belong to enum {adt:?}"
            ))
        })?;
        let enum_type = self.mapper.adt(adt, args)?;
        let storage = self.entry_alloca(enum_type.into(), "enum.value")?;

        let tag_bits = u16::try_from(layout.tag.size() * 8).map_err(|_| {
            CodegenError::internal(format!("enum {adt:?} has a tag too wide for codegen"))
        })?;
        let tag_type = self.mapper.integer_type(tag_bits)?;
        let tag = self.int_constant(tag_type, u128::from(variant.index()));
        let tag_ptr = self.build(
            self.builder
                .build_struct_gep(enum_type, storage, 0, "enum.tag"),
            "enum tag projection",
        )?;
        self.build(self.builder.build_store(tag_ptr, tag), "enum tag store")?;

        let mut values = FxHashMap::default();
        for (field, operand) in fields {
            values.insert(*field, self.load_operand(operand)?);
        }

        let mut field_types = Vec::with_capacity(variant_def.fields.len());
        for field in &variant_def.fields {
            let field_ty = self.tcx.substitute(field.ty, args);
            field_types.push(self.mapper.basic(field_ty)?);
        }
        let variant_type = self.context.struct_type(&field_types, false);
        let mut payload = variant_type.get_undef();
        for (index, field) in variant_def.fields.iter().enumerate() {
            let value = values.remove(&field.def).ok_or_else(|| {
                CodegenError::internal(format!(
                    "variant {variant:?} is missing an initializer for {field:?}"
                ))
            })?;
            let next = self.build(
                self.builder
                    .build_insert_value(payload, value, index as u32, "variant.field"),
                "variant field",
            )?;
            payload = match next {
                inkwell::values::AggregateValueEnum::StructValue(value) => value,
                other => {
                    return Err(CodegenError::internal(format!(
                        "variant construction produced {other:?}"
                    )));
                }
            };
        }
        let payload_ptr = self.build(
            self.builder
                .build_struct_gep(enum_type, storage, 2, "enum.payload"),
            "enum payload projection",
        )?;
        self.build(
            self.builder.build_store(payload_ptr, payload),
            "enum payload store",
        )?;

        let value = self.build(
            self.builder.build_load(enum_type, storage, "enum.value"),
            "enum load",
        )?;
        Ok(value)
    }
}
