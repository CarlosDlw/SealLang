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
use seal_ty::TyKind;

use crate::error::CodegenError;
use crate::function::FunctionLowerer;

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    /// Evaluates `rvalue` and returns its value.
    pub(crate) fn emit_rvalue(
        &self,
        rvalue: &Rvalue,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        match rvalue {
            Rvalue::Use(operand) => self.load_operand(operand),
            Rvalue::Ref(place) => Ok(self.place_ptr(place)?.into()),
            Rvalue::Binary { op, lhs, rhs } => {
                let ty = self.operand_type(lhs)?;
                let lhs = self.load_operand(lhs)?;
                let rhs = self.load_operand(rhs)?;
                self.binary(*op, lhs, rhs, ty, span)
            }
            Rvalue::Unary { op, operand } => {
                let value = self.load_operand(operand)?;
                self.unary(*op, value)
            }
            Rvalue::Call { func, args } => self.call(*func, args),
            Rvalue::Aggregate { kind, fields } => self.aggregate(*kind, fields),
        }
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
        func: DefId,
        arguments: &[seal_mir::Operand],
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let function = self
            .module
            .get_function(&crate::names::function(
                self.interner.resolve(self.definitions().get(func).name),
            ))
            .ok_or_else(|| {
                CodegenError::internal(format!("call target {func:?} was not declared"))
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
        let AggregateKind::Adt(def) = kind;
        let adt = self.adt_of(def)?;
        let definition = self.tcx.adt(adt);
        let struct_type = self.mapper.adt(adt);

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
}
