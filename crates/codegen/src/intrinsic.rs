//! Code generation for compiler-owned intrinsic methods.
//!
//! Scalar methods map to LLVM instructions and target intrinsics. Sequence
//! methods (`swap`, `reverse`, `fill`) check every runtime index, then emit
//! loops over the data pointer. None of these paths allocate outside the
//! function entry block, so calling a method inside a loop does not grow the
//! stack.

use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, IntType};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValueEnum, FloatValue, FunctionValue, IntValue, PointerValue,
};
use inkwell::{FloatPredicate, IntPredicate};
use seal_methods::{IntrinsicOp, MethodId, MethodOwner};
use seal_mir::{IntrinsicReceiver, Operand};
use seal_span::Span;
use seal_ty::TyKind;

use crate::error::CodegenError;
use crate::function::FunctionLowerer;

/// A sequence receiver split into its elements pointer and runtime length.
struct Sequence<'ctx> {
    data: PointerValue<'ctx>,
    length: IntValue<'ctx>,
    element: seal_ty::TyId,
}

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    pub(crate) fn emit_intrinsic(
        &self,
        method: MethodId,
        receiver: &IntrinsicReceiver,
        args: &[Operand],
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let definition = method.def();
        match definition.owner {
            MethodOwner::Array | MethodOwner::Slice => {
                self.sequence_intrinsic(definition.intrinsic, receiver, args, span)
            }
            MethodOwner::RawPtr => {
                self.raw_pointer_intrinsic(definition.intrinsic, receiver, args, span)
            }
            _ => {
                self.scalar_intrinsic(definition.owner, definition.intrinsic, receiver, args, span)
            }
        }
    }

    fn scalar_intrinsic(
        &self,
        owner: MethodOwner,
        intrinsic: IntrinsicOp,
        receiver: &IntrinsicReceiver,
        args: &[Operand],
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        if intrinsic == IntrinsicOp::BoolToggle {
            let IntrinsicReceiver::Place(place) = receiver else {
                return Err(CodegenError::internal(
                    "`bool::toggle` has a non-place receiver",
                ));
            };
            let pointer = self.place_ptr(place)?;
            let bool_ty = self.tcx.bool_ty();
            let value = self
                .build(
                    self.builder
                        .build_load(self.mapper.basic(bool_ty)?, pointer, "toggle value"),
                    "toggle load",
                )?
                .into_int_value();
            let toggled = self.build(
                self.builder.build_xor(
                    value,
                    self.mapper.basic(bool_ty)?.into_int_type().const_all_ones(),
                    "toggle",
                ),
                "toggle",
            )?;
            self.build(self.builder.build_store(pointer, toggled), "toggle store")?;
            return Ok(self.unit_value());
        }

        let value = match receiver {
            IntrinsicReceiver::Place(place) => self.load_operand(&Operand::Copy(place.clone()))?,
            IntrinsicReceiver::Value(operand) => self.load_operand(operand)?,
        };
        match intrinsic {
            IntrinsicOp::IntAbs => {
                let value = value.into_int_value();
                let int_ty = value.get_type();
                let function = self.intrinsic_function(
                    &format!("llvm.abs.i{}", int_ty.get_bit_width()),
                    &[int_ty.into(), self.context.bool_type().into()],
                    int_ty.into(),
                )?;
                self.call_intrinsic(
                    function,
                    &[value.into(), self.context.bool_type().const_zero().into()],
                )
            }
            IntrinsicOp::IntSignum => self.int_signum(value.into_int_value()),
            IntrinsicOp::IntMin => {
                let rhs = self.load_operand(&args[0])?.into_int_value();
                self.int_min_max(value.into_int_value(), rhs, true, owner == MethodOwner::Int)
            }
            IntrinsicOp::IntMax => {
                let rhs = self.load_operand(&args[0])?.into_int_value();
                self.int_min_max(
                    value.into_int_value(),
                    rhs,
                    false,
                    owner == MethodOwner::Int,
                )
            }
            IntrinsicOp::IntClamp => {
                let minimum = self.load_operand(&args[0])?.into_int_value();
                let maximum = self.load_operand(&args[1])?.into_int_value();
                self.int_clamp(
                    value.into_int_value(),
                    minimum,
                    maximum,
                    span,
                    owner == MethodOwner::Int,
                )
            }
            IntrinsicOp::IntWrappingAdd
            | IntrinsicOp::IntWrappingSub
            | IntrinsicOp::IntWrappingMul => {
                let rhs = self.load_operand(&args[0])?.into_int_value();
                let lhs = value.into_int_value();
                let result = match intrinsic {
                    IntrinsicOp::IntWrappingAdd => {
                        self.builder.build_int_add(lhs, rhs, "wrapping add")
                    }
                    IntrinsicOp::IntWrappingSub => {
                        self.builder.build_int_sub(lhs, rhs, "wrapping sub")
                    }
                    _ => self.builder.build_int_mul(lhs, rhs, "wrapping mul"),
                };
                Ok(self.build(result, "wrapping operation")?.into())
            }
            IntrinsicOp::IntSaturatingAdd
            | IntrinsicOp::IntSaturatingSub
            | IntrinsicOp::IntSaturatingMul => {
                let rhs = self.load_operand(&args[0])?.into_int_value();
                self.saturating(
                    intrinsic,
                    value.into_int_value(),
                    rhs,
                    owner == MethodOwner::Int,
                )
            }
            IntrinsicOp::IntCountOnes => {
                let value = value.into_int_value();
                let result = self.int_unary_intrinsic("ctpop", value)?.into_int_value();
                Ok(self.resize_int(result, self.i32_type()?)?.into())
            }
            IntrinsicOp::IntLeadingZeros => {
                let value = value.into_int_value();
                let result = self
                    .int_leading_trailing_zeros("ctlz", value)?
                    .into_int_value();
                Ok(self.resize_int(result, self.i32_type()?)?.into())
            }
            IntrinsicOp::IntTrailingZeros => {
                let value = value.into_int_value();
                let result = self
                    .int_leading_trailing_zeros("cttz", value)?
                    .into_int_value();
                Ok(self.resize_int(result, self.i32_type()?)?.into())
            }
            IntrinsicOp::IntRotateLeft | IntrinsicOp::IntRotateRight => {
                let amount = self.load_operand(&args[0])?.into_int_value();
                let value = value.into_int_value();
                let int_ty = value.get_type();
                let name = if intrinsic == IntrinsicOp::IntRotateLeft {
                    "fshl"
                } else {
                    "fshr"
                };
                let function = self.intrinsic_function(
                    &format!("llvm.{name}.i{}", int_ty.get_bit_width()),
                    &[int_ty.into(), int_ty.into(), int_ty.into()],
                    int_ty.into(),
                )?;
                let amount = self.resize_int(amount, int_ty)?;
                self.call_intrinsic(function, &[value.into(), value.into(), amount.into()])
            }
            IntrinsicOp::IntSwapBytes => {
                let value = value.into_int_value();
                if value.get_type().get_bit_width() <= 8 {
                    return Ok(value.into());
                }
                self.int_unary_intrinsic("bswap", value)
            }
            IntrinsicOp::FloatAbs | IntrinsicOp::FloatSqrt => {
                let value = value.into_float_value();
                let name = if intrinsic == IntrinsicOp::FloatAbs {
                    "fabs"
                } else {
                    "sqrt"
                };
                self.float_unary_intrinsic(name, value)
            }
            IntrinsicOp::FloatFloor
            | IntrinsicOp::FloatCeil
            | IntrinsicOp::FloatRound
            | IntrinsicOp::FloatTrunc => self.float_rounding(intrinsic, value.into_float_value()),
            IntrinsicOp::FloatFract => {
                let value = value.into_float_value();
                let truncated = self.float_trunc(value)?;
                Ok(self
                    .build(
                        self.builder.build_float_sub(value, truncated, "fract"),
                        "float fraction",
                    )?
                    .into())
            }
            IntrinsicOp::FloatPowi => {
                let value = value.into_float_value();
                let exponent = self.load_operand(&args[0])?.into_int_value();
                let float_ty = value.get_type();
                let float_name = self.float_type_name(float_ty)?;
                let function = self.intrinsic_function(
                    &format!("llvm.powi.{float_name}.i32"),
                    &[float_ty.into(), self.i32_type()?.into()],
                    float_ty.into(),
                )?;
                let exponent = self.resize_int(exponent, self.i32_type()?)?;
                self.call_intrinsic(function, &[value.into(), exponent.into()])
            }
            IntrinsicOp::FloatMin => {
                let rhs = self.load_operand(&args[0])?.into_float_value();
                self.float_min_max(value.into_float_value(), rhs, true)
            }
            IntrinsicOp::FloatMax => {
                let rhs = self.load_operand(&args[0])?.into_float_value();
                self.float_min_max(value.into_float_value(), rhs, false)
            }
            IntrinsicOp::FloatClamp => {
                let minimum = self.load_operand(&args[0])?.into_float_value();
                let maximum = self.load_operand(&args[1])?.into_float_value();
                self.float_clamp(value.into_float_value(), minimum, maximum, span)
            }
            IntrinsicOp::FloatIsNan | IntrinsicOp::FloatIsFinite | IntrinsicOp::FloatIsInfinite => {
                let value = value.into_float_value();
                self.float_predicate(intrinsic, value)
            }
            IntrinsicOp::BoolNot => {
                let value = value.into_int_value();
                let int_ty = value.get_type();
                Ok(self
                    .build(
                        self.builder
                            .build_xor(value, int_ty.const_all_ones(), "boolean not"),
                        "boolean not",
                    )?
                    .into())
            }
            IntrinsicOp::BoolToI32 => {
                let value = value.into_int_value();
                Ok(self.resize_int(value, self.i32_type()?)?.into())
            }
            IntrinsicOp::CharIsAscii
            | IntrinsicOp::CharIsAsciiAlphabetic
            | IntrinsicOp::CharIsAsciiDigit
            | IntrinsicOp::CharIsAsciiWhitespace => {
                let value = value.into_int_value();
                self.char_predicate(intrinsic, value)
            }
            IntrinsicOp::CharToAsciiUppercase | IntrinsicOp::CharToAsciiLowercase => {
                let value = value.into_int_value();
                self.char_case(intrinsic, value)
            }
            IntrinsicOp::CharLenUtf8 => {
                let value = value.into_int_value();
                let usize = self.mapper.usize_type()?;
                let value = self.resize_int(value, self.i32_type()?)?;
                let at_most = |limit: u64| -> Result<IntValue<'ctx>, CodegenError> {
                    self.build(
                        self.builder.build_int_compare(
                            IntPredicate::ULE,
                            value,
                            self.i32_type()?.const_int(limit, false),
                            "utf8 limit",
                        ),
                        "utf8 comparison",
                    )
                };
                let one = usize.const_int(1, false);
                let two = usize.const_int(2, false);
                let three = usize.const_int(3, false);
                let four = usize.const_int(4, false);
                let mut result = four;
                // Start at four bytes and narrow downwards: the first
                // satisfied limit must win, not the widest one.
                for (condition, replacement) in [
                    (at_most(0xffff)?, three),
                    (at_most(0x7ff)?, two),
                    (at_most(0x7f)?, one),
                ] {
                    result = self
                        .build(
                            self.builder.build_select(
                                condition,
                                replacement,
                                result,
                                "utf8 length",
                            ),
                            "utf8 select",
                        )?
                        .into_int_value();
                }
                Ok(result.into())
            }
            IntrinsicOp::CharToU32 => Ok(value.into_int_value().into()),
            IntrinsicOp::StrLen => {
                let fat = value.into_struct_value();
                let length = self
                    .build(
                        self.builder.build_extract_value(fat, 1, "string length"),
                        "string length",
                    )?
                    .into_int_value();
                Ok(length.into())
            }
            IntrinsicOp::StrIsCharBoundary => {
                let [index] = args else {
                    return Err(CodegenError::internal(
                        "`str::is_char_boundary` takes exactly one index",
                    ));
                };
                let fat = value.into_struct_value();
                let data = self
                    .build(
                        self.builder.build_extract_value(fat, 0, "string data"),
                        "string data",
                    )?
                    .into_pointer_value();
                let length = self
                    .build(
                        self.builder.build_extract_value(fat, 1, "string length"),
                        "string length",
                    )?
                    .into_int_value();
                let index = self.load_operand(index)?.into_int_value();
                let at_end = self.build(
                    self.builder.build_int_compare(
                        IntPredicate::EQ,
                        index,
                        length,
                        "boundary at end",
                    ),
                    "boundary comparison",
                )?;
                let in_bounds = self.build(
                    self.builder.build_int_compare(
                        IntPredicate::ULT,
                        index,
                        length,
                        "boundary in bounds",
                    ),
                    "boundary comparison",
                )?;
                let current = self.builder.get_insert_block().ok_or_else(|| {
                    CodegenError::internal("a string boundary check has no insertion block")
                })?;
                let check = self
                    .context
                    .append_basic_block(self.function, "str.boundary.check");
                let merged = self
                    .context
                    .append_basic_block(self.function, "str.boundary.merged");
                self.build(
                    self.builder
                        .build_conditional_branch(in_bounds, check, merged),
                    "boundary branch",
                )?;
                self.builder.position_at_end(check);
                let bytes = self.tcx.uint_ty(seal_ty::UintTy::U8);
                let byte_ty = self.mapper.basic(bytes)?.into_int_type();
                let pointer = self.index_ptr(data, index, bytes)?;
                let byte = self
                    .build(
                        self.builder.build_load(byte_ty, pointer, "str byte"),
                        "string byte load",
                    )?
                    .into_int_value();
                let masked = self.build(
                    self.builder
                        .build_and(byte, byte_ty.const_int(0xc0, false), "utf8 prefix"),
                    "utf8 prefix mask",
                )?;
                let boundary = self.build(
                    self.builder.build_int_compare(
                        IntPredicate::NE,
                        masked,
                        byte_ty.const_int(0x80, false),
                        "boundary byte",
                    ),
                    "boundary comparison",
                )?;
                self.build(
                    self.builder.build_unconditional_branch(merged),
                    "boundary continue",
                )?;
                self.builder.position_at_end(merged);
                let bool_ty = self.mapper.basic(self.tcx.bool_ty())?.into_int_type();
                let phi = self.build(
                    self.builder.build_phi(bool_ty, "str boundary"),
                    "boundary phi",
                )?;
                phi.add_incoming(&[(&boundary, check), (&at_end, current)]);
                Ok(phi.as_basic_value())
            }
            IntrinsicOp::BoolToggle => Err(CodegenError::internal(
                "`bool::toggle` reached the value path after its special case",
            )),
            IntrinsicOp::Len
            | IntrinsicOp::StrCharAt
            | IntrinsicOp::StrSlice
            | IntrinsicOp::StrAsBytes
            | IntrinsicOp::SliceAsStrUnchecked
            | IntrinsicOp::IsEmpty
            | IntrinsicOp::Swap
            | IntrinsicOp::Reverse
            | IntrinsicOp::Fill
            | IntrinsicOp::PtrAdd
            | IntrinsicOp::PtrSub
            | IntrinsicOp::PtrOffset
            | IntrinsicOp::PtrByteAdd
            | IntrinsicOp::PtrByteOffset
            | IntrinsicOp::PtrOffsetFrom
            | IntrinsicOp::PtrWrappingAdd
            | IntrinsicOp::PtrWrappingSub
            | IntrinsicOp::PtrWrappingOffset
            | IntrinsicOp::PtrIsNull
            | IntrinsicOp::PtrAddr
            | IntrinsicOp::PtrExposeAddr
            | IntrinsicOp::PtrWithAddr
            | IntrinsicOp::PtrLen
            | IntrinsicOp::PtrData
            | IntrinsicOp::PtrWithLen
            | IntrinsicOp::PtrAsStr
            | IntrinsicOp::PtrRead
            | IntrinsicOp::PtrWrite
            | IntrinsicOp::PtrReadUnaligned
            | IntrinsicOp::PtrWriteUnaligned
            | IntrinsicOp::PtrCopy
            | IntrinsicOp::PtrCopyNonOverlapping => Err(CodegenError::internal(
                "a raw or sequence intrinsic reached scalar code generation",
            )),
        }
    }

    fn sequence_intrinsic(
        &self,
        intrinsic: IntrinsicOp,
        receiver: &IntrinsicReceiver,
        args: &[Operand],
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let sequence = self.sequence(receiver)?;
        match intrinsic {
            IntrinsicOp::Len => Ok(sequence.length.into()),
            IntrinsicOp::IsEmpty => {
                let bool_ty = self.mapper.basic(self.tcx.bool_ty())?.into_int_type();
                let empty = self.build(
                    self.builder.build_int_compare(
                        IntPredicate::EQ,
                        sequence.length,
                        sequence.length.get_type().const_zero(),
                        "is_empty",
                    ),
                    "empty comparison",
                )?;
                let _ = bool_ty;
                Ok(empty.into())
            }
            IntrinsicOp::Swap => {
                let first = self.load_operand(&args[0])?.into_int_value();
                let second = self.load_operand(&args[1])?.into_int_value();
                self.bounds_check(first, sequence.length, span)?;
                self.bounds_check(second, sequence.length, span)?;
                self.swap_at(&sequence, first, second)?;
                Ok(self.unit_value())
            }
            IntrinsicOp::Reverse => {
                self.reverse(&sequence, span)?;
                Ok(self.unit_value())
            }
            IntrinsicOp::Fill => {
                let value = self.load_operand(&args[0])?;
                self.fill(&sequence, value)?;
                Ok(self.unit_value())
            }
            _ => Err(CodegenError::internal(
                "a scalar intrinsic reached sequence code generation",
            )),
        }
    }

    fn sequence(&self, receiver: &IntrinsicReceiver) -> Result<Sequence<'ctx>, CodegenError> {
        match receiver {
            IntrinsicReceiver::Place(place) => {
                let ty = self.place_type(place)?;
                match self.tcx.kind(ty) {
                    TyKind::Array { element, length } => Ok(Sequence {
                        data: self.place_ptr(place)?,
                        length: self.mapper.usize_type()?.const_int(length, false),
                        element,
                    }),
                    TyKind::Ref { pointee, .. } => match self.tcx.kind(pointee) {
                        TyKind::Slice { element } => {
                            let fat = self
                                .build(
                                    self.builder.build_load(
                                        self.mapper.basic(ty)?,
                                        self.place_ptr(place)?,
                                        "slice receiver",
                                    ),
                                    "slice receiver load",
                                )?
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
                            Ok(Sequence {
                                data,
                                length,
                                element,
                            })
                        }
                        _ => Err(CodegenError::internal(format!(
                            "method receiver `{ty:?}` is not an array or slice"
                        ))),
                    },
                    _ => Err(CodegenError::internal(format!(
                        "method receiver `{ty:?}` is not an array or slice"
                    ))),
                }
            }
            IntrinsicReceiver::Value(operand) => {
                let ty = self.operand_type(operand)?;
                let TyKind::Ref { pointee, .. } = self.tcx.kind(ty) else {
                    return Err(CodegenError::internal(format!(
                        "slice receiver `{ty:?}` is not a reference"
                    )));
                };
                let TyKind::Slice { element } = self.tcx.kind(pointee) else {
                    return Err(CodegenError::internal(format!(
                        "slice receiver `{ty:?}` does not point to a slice"
                    )));
                };
                let fat = self.load_operand(operand)?.into_struct_value();
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
                Ok(Sequence {
                    data,
                    length,
                    element,
                })
            }
        }
    }

    fn int_signum(&self, value: IntValue<'ctx>) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let ty = value.get_type();
        let zero = ty.const_zero();
        let one = ty.const_int(1, false);
        let minus_one = ty.const_all_ones();
        let positive = self.build(
            self.builder
                .build_int_compare(IntPredicate::SGT, value, zero, "sign positive"),
            "sign comparison",
        )?;
        let negative = self.build(
            self.builder
                .build_int_compare(IntPredicate::SLT, value, zero, "sign negative"),
            "sign comparison",
        )?;
        let non_negative = self
            .build(
                self.builder
                    .build_select(positive, one, zero, "sign non-negative"),
                "sign select",
            )?
            .into_int_value();
        let result = self.build(
            self.builder
                .build_select(negative, minus_one, non_negative, "sign"),
            "sign select",
        )?;
        Ok(result)
    }

    fn int_min_max(
        &self,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
        minimum: bool,
        signed: bool,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let predicate = match (minimum, signed) {
            (true, true) => IntPredicate::SLT,
            (true, false) => IntPredicate::ULT,
            (false, true) => IntPredicate::SGT,
            (false, false) => IntPredicate::UGT,
        };
        let condition = self.build(
            self.builder
                .build_int_compare(predicate, lhs, rhs, "min_max"),
            "min/max comparison",
        )?;
        self.build(
            self.builder.build_select(condition, lhs, rhs, "min_max"),
            "min/max select",
        )
    }

    fn int_clamp(
        &self,
        value: IntValue<'ctx>,
        minimum: IntValue<'ctx>,
        maximum: IntValue<'ctx>,
        span: Span,
        signed: bool,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let ordered = self.build(
            self.builder.build_int_compare(
                if signed {
                    IntPredicate::SLE
                } else {
                    IntPredicate::ULE
                },
                minimum,
                maximum,
                "clamp bounds",
            ),
            "clamp comparison",
        )?;
        self.check_or_panic(ordered, span, "`clamp` requires minimum <= maximum")?;
        let below = self.build(
            self.builder.build_int_compare(
                if signed {
                    IntPredicate::SLT
                } else {
                    IntPredicate::ULT
                },
                value,
                minimum,
                "clamp below",
            ),
            "clamp comparison",
        )?;
        let above = self.build(
            self.builder.build_int_compare(
                if signed {
                    IntPredicate::SGT
                } else {
                    IntPredicate::UGT
                },
                value,
                maximum,
                "clamp above",
            ),
            "clamp comparison",
        )?;
        let lower = self
            .build(
                self.builder
                    .build_select(below, minimum, value, "clamp lower"),
                "clamp select",
            )?
            .into_int_value();
        self.build(
            self.builder.build_select(above, maximum, lower, "clamp"),
            "clamp select",
        )
    }

    fn saturating(
        &self,
        intrinsic: IntrinsicOp,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
        signed: bool,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let int_ty = lhs.get_type();
        let base = match intrinsic {
            IntrinsicOp::IntSaturatingAdd => "add",
            IntrinsicOp::IntSaturatingSub => "sub",
            IntrinsicOp::IntSaturatingMul => "mul",
            _ => {
                return Err(CodegenError::internal(
                    "a non-saturating operation reached saturation",
                ));
            }
        };
        let prefix = if signed { "s" } else { "u" };
        let bool_ty = self.context.bool_type();
        let pair_ty = self
            .context
            .struct_type(&[int_ty.into(), bool_ty.into()], false);
        let function = self.intrinsic_function(
            &format!(
                "llvm.{prefix}{base}.with.overflow.i{}",
                int_ty.get_bit_width()
            ),
            &[int_ty.into(), int_ty.into()],
            pair_ty.into(),
        )?;
        let pair = self
            .call_intrinsic(function, &[lhs.into(), rhs.into()])?
            .into_struct_value();
        let result = self
            .build(
                self.builder
                    .build_extract_value(pair, 0, "saturating result"),
                "saturating result",
            )?
            .into_int_value();
        let overflow = self
            .build(
                self.builder
                    .build_extract_value(pair, 1, "saturating overflow"),
                "saturating overflow",
            )?
            .into_int_value();
        if !signed {
            let limit = if base == "sub" {
                int_ty.const_zero()
            } else {
                int_ty.const_all_ones()
            };
            return self.build(
                self.builder
                    .build_select(overflow, limit, result, "saturating unsigned"),
                "saturating select",
            );
        }

        let zero = int_ty.const_zero();
        let positive = match base {
            "add" => self.build(
                self.builder
                    .build_int_compare(IntPredicate::SGE, rhs, zero, "saturating sign"),
                "saturating sign",
            )?,
            "sub" => self.build(
                self.builder
                    .build_int_compare(IntPredicate::SLT, rhs, zero, "saturating sign"),
                "saturating sign",
            )?,
            _ => {
                let lhs_positive = self.build(
                    self.builder
                        .build_int_compare(IntPredicate::SGE, lhs, zero, "lhs sign"),
                    "saturating sign",
                )?;
                let rhs_positive = self.build(
                    self.builder
                        .build_int_compare(IntPredicate::SGE, rhs, zero, "rhs sign"),
                    "saturating sign",
                )?;
                let different = self.build(
                    self.builder
                        .build_xor(lhs_positive, rhs_positive, "saturating sign"),
                    "saturating sign",
                )?;
                self.build(
                    self.builder.build_not(different, "saturating sign"),
                    "saturating sign",
                )?
            }
        };
        let width = int_ty.get_bit_width();
        let maximum = if width > 64 {
            int_ty.const_int_arbitrary_precision(&[u64::MAX, u64::MAX >> 1])
        } else {
            int_ty.const_int((1u64 << (width - 1)) - 1, false)
        };
        let minimum = if width > 64 {
            int_ty.const_int_arbitrary_precision(&[0, 1u64 << 63])
        } else {
            int_ty.const_int(1u64 << (width - 1), false)
        };
        let limit = self
            .build(
                self.builder
                    .build_select(positive, maximum, minimum, "saturating limit"),
                "saturating limit",
            )?
            .into_int_value();
        self.build(
            self.builder
                .build_select(overflow, limit, result, "saturating signed"),
            "saturating select",
        )
    }

    fn int_unary_intrinsic(
        &self,
        name: &str,
        value: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let int_ty = value.get_type();
        let function = self.intrinsic_function(
            &format!("llvm.{name}.i{}", int_ty.get_bit_width()),
            &[int_ty.into()],
            int_ty.into(),
        )?;
        self.call_intrinsic(function, &[value.into()])
    }

    fn int_leading_trailing_zeros(
        &self,
        name: &str,
        value: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let int_ty = value.get_type();
        let function = self.intrinsic_function(
            &format!("llvm.{name}.i{}", int_ty.get_bit_width()),
            &[int_ty.into(), self.context.bool_type().into()],
            int_ty.into(),
        )?;
        self.call_intrinsic(
            function,
            &[value.into(), self.context.bool_type().const_zero().into()],
        )
    }

    fn float_min_max(
        &self,
        lhs: FloatValue<'ctx>,
        rhs: FloatValue<'ctx>,
        minimum: bool,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let lhs_nan = self.build(
            self.builder
                .build_float_compare(FloatPredicate::UNO, lhs, lhs, "lhs_nan"),
            "NaN comparison",
        )?;
        let rhs_nan = self.build(
            self.builder
                .build_float_compare(FloatPredicate::UNO, rhs, rhs, "rhs_nan"),
            "NaN comparison",
        )?;
        let ordered = self.build(
            self.builder.build_float_compare(
                if minimum {
                    FloatPredicate::OLT
                } else {
                    FloatPredicate::OGT
                },
                lhs,
                rhs,
                "min_max",
            ),
            "min/max comparison",
        )?;
        let chosen = self
            .build(
                self.builder.build_select(ordered, lhs, rhs, "min_max"),
                "min/max select",
            )?
            .into_float_value();
        let chosen = self
            .build(
                self.builder.build_select(rhs_nan, lhs, chosen, "rhs_nan"),
                "NaN select",
            )?
            .into_float_value();
        self.build(
            self.builder.build_select(lhs_nan, rhs, chosen, "lhs_nan"),
            "NaN select",
        )
    }

    fn float_clamp(
        &self,
        value: FloatValue<'ctx>,
        minimum: FloatValue<'ctx>,
        maximum: FloatValue<'ctx>,
        span: Span,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let ordered = self.build(
            self.builder
                .build_float_compare(FloatPredicate::OLE, minimum, maximum, "clamp bounds"),
            "clamp comparison",
        )?;
        self.check_or_panic(ordered, span, "`clamp` requires minimum <= maximum")?;
        let below = self.build(
            self.builder
                .build_float_compare(FloatPredicate::OLT, value, minimum, "clamp below"),
            "clamp comparison",
        )?;
        let above = self.build(
            self.builder
                .build_float_compare(FloatPredicate::OGT, value, maximum, "clamp above"),
            "clamp comparison",
        )?;
        let lower = self
            .build(
                self.builder
                    .build_select(below, minimum, value, "clamp lower"),
                "clamp select",
            )?
            .into_float_value();
        self.build(
            self.builder.build_select(above, maximum, lower, "clamp"),
            "clamp select",
        )
    }

    fn float_predicate(
        &self,
        intrinsic: IntrinsicOp,
        value: FloatValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let comparison = match intrinsic {
            IntrinsicOp::FloatIsNan => {
                self.builder
                    .build_float_compare(FloatPredicate::UNO, value, value, "is_nan")
            }
            IntrinsicOp::FloatIsFinite => {
                let abs = self.float_unary_intrinsic("fabs", value)?;
                let infinity = value.get_type().const_float(f64::INFINITY);
                self.builder.build_float_compare(
                    FloatPredicate::OLT,
                    abs.into_float_value(),
                    infinity,
                    "is_finite",
                )
            }
            IntrinsicOp::FloatIsInfinite => {
                let abs = self.float_unary_intrinsic("fabs", value)?;
                let infinity = value.get_type().const_float(f64::INFINITY);
                self.builder.build_float_compare(
                    FloatPredicate::OEQ,
                    abs.into_float_value(),
                    infinity,
                    "is_infinite",
                )
            }
            _ => unreachable!(),
        };
        Ok(self.build(comparison, "float predicate")?.into())
    }

    fn char_predicate(
        &self,
        intrinsic: IntrinsicOp,
        value: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let i32_ty = self.i32_type()?;
        let compare = |predicate: IntPredicate,
                       lhs: IntValue<'ctx>,
                       rhs: IntValue<'ctx>,
                       name: &str|
         -> Result<IntValue<'ctx>, CodegenError> {
            self.build(
                self.builder.build_int_compare(predicate, lhs, rhs, name),
                "character comparison",
            )
        };
        let result = match intrinsic {
            IntrinsicOp::CharIsAscii => compare(
                IntPredicate::ULE,
                value,
                i32_ty.const_int(0x7f, false),
                "is_ascii",
            )?,
            IntrinsicOp::CharIsAsciiAlphabetic => {
                let upper = compare(
                    IntPredicate::UGE,
                    value,
                    i32_ty.const_int(u64::from(b'A'), false),
                    "at_least_A",
                )?;
                let lower = compare(
                    IntPredicate::ULE,
                    value,
                    i32_ty.const_int(u64::from(b'Z'), false),
                    "at_most_Z",
                )?;
                let uppercase = self.build(
                    self.builder.build_and(upper, lower, "uppercase"),
                    "ASCII range",
                )?;
                let lower_a = compare(
                    IntPredicate::UGE,
                    value,
                    i32_ty.const_int(u64::from(b'a'), false),
                    "at_least_a",
                )?;
                let lower_z = compare(
                    IntPredicate::ULE,
                    value,
                    i32_ty.const_int(u64::from(b'z'), false),
                    "at_most_z",
                )?;
                let lowercase = self.build(
                    self.builder.build_and(lower_a, lower_z, "lowercase"),
                    "ASCII range",
                )?;
                self.build(
                    self.builder
                        .build_or(uppercase, lowercase, "is_ascii_alphabetic"),
                    "ASCII range",
                )?
            }
            IntrinsicOp::CharIsAsciiDigit => {
                let lower = compare(
                    IntPredicate::UGE,
                    value,
                    i32_ty.const_int(u64::from(b'0'), false),
                    "at_least_zero",
                )?;
                let upper = compare(
                    IntPredicate::ULE,
                    value,
                    i32_ty.const_int(u64::from(b'9'), false),
                    "at_most_nine",
                )?;
                self.build(
                    self.builder.build_and(lower, upper, "is_ascii_digit"),
                    "ASCII range",
                )?
            }
            IntrinsicOp::CharIsAsciiWhitespace => {
                let space = compare(
                    IntPredicate::EQ,
                    value,
                    i32_ty.const_int(u64::from(b' '), false),
                    "space",
                )?;
                let tab = compare(
                    IntPredicate::UGE,
                    value,
                    i32_ty.const_int(0x09, false),
                    "at_least_tab",
                )?;
                let carriage_return = compare(
                    IntPredicate::ULE,
                    value,
                    i32_ty.const_int(0x0d, false),
                    "at_most_cr",
                )?;
                let control = self.build(
                    self.builder.build_and(tab, carriage_return, "ascii_space"),
                    "ASCII range",
                )?;
                self.build(
                    self.builder.build_or(space, control, "is_ascii_whitespace"),
                    "ASCII range",
                )?
            }
            _ => {
                return Err(CodegenError::internal(
                    "a non-predicate operation reached character predicates",
                ));
            }
        };
        Ok(result.into())
    }

    fn char_case(
        &self,
        intrinsic: IntrinsicOp,
        value: IntValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let i32_ty = self.i32_type()?;
        let (minimum, maximum, delta) = if intrinsic == IntrinsicOp::CharToAsciiUppercase {
            (u64::from(b'a'), u64::from(b'z'), -32i32)
        } else {
            (u64::from(b'A'), u64::from(b'Z'), 32i32)
        };
        let at_least = self.build(
            self.builder.build_int_compare(
                IntPredicate::UGE,
                value,
                i32_ty.const_int(minimum, false),
                "case lower bound",
            ),
            "case comparison",
        )?;
        let at_most = self.build(
            self.builder.build_int_compare(
                IntPredicate::ULE,
                value,
                i32_ty.const_int(maximum, false),
                "case upper bound",
            ),
            "case comparison",
        )?;
        let in_range = self.build(
            self.builder.build_and(at_least, at_most, "case range"),
            "case range",
        )?;
        let shifted = self.build(
            self.builder
                .build_int_add(value, i32_ty.const_int(delta as u64, false), "case shift"),
            "case shift",
        )?;
        self.build(
            self.builder.build_select(in_range, shifted, value, "case"),
            "case select",
        )
    }

    fn resize_int(
        &self,
        value: IntValue<'ctx>,
        destination: IntType<'ctx>,
    ) -> Result<IntValue<'ctx>, CodegenError> {
        let source_width = value.get_type().get_bit_width();
        let destination_width = destination.get_bit_width();
        if source_width == destination_width {
            return Ok(value);
        }
        if source_width < destination_width {
            self.build(
                self.builder
                    .build_int_z_extend(value, destination, "extend"),
                "integer extension",
            )
        } else {
            self.build(
                self.builder
                    .build_int_truncate(value, destination, "truncate"),
                "integer truncation",
            )
        }
    }

    fn float_unary_intrinsic(
        &self,
        name: &str,
        value: FloatValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let float_ty = value.get_type();
        let float_name = self.float_type_name(float_ty)?;
        let function = self.intrinsic_function(
            &format!("llvm.{name}.{float_name}"),
            &[float_ty.into()],
            float_ty.into(),
        )?;
        self.call_intrinsic(function, &[value.into()])
    }

    /// Truncates toward zero without libm, including values outside the range
    /// of an integer conversion.
    fn float_trunc(&self, value: FloatValue<'ctx>) -> Result<FloatValue<'ctx>, CodegenError> {
        let float_ty = value.get_type();
        let width = float_ty.get_bit_width();
        let (integer_ty, threshold) = match width {
            32 => (self.context.i32_type(), 8_388_608.0f64),
            64 => (self.context.i64_type(), 4_503_599_627_370_496.0f64),
            _ => {
                return Err(CodegenError::internal(format!(
                    "unsupported floating-point width `{width}`"
                )));
            }
        };
        let zero = float_ty.const_float(0.0);
        let one = float_ty.const_float(1.0);
        let threshold = float_ty.const_float(threshold);
        let abs = self
            .float_unary_intrinsic("fabs", value)?
            .into_float_value();
        let is_nan = self.build(
            self.builder
                .build_float_compare(FloatPredicate::UNO, value, value, "trunc_nan"),
            "trunc NaN",
        )?;
        let already_integer = self.build(
            self.builder
                .build_float_compare(FloatPredicate::OGE, abs, threshold, "trunc_large"),
            "trunc range",
        )?;
        let below_one = self.build(
            self.builder
                .build_float_compare(FloatPredicate::OLT, abs, one, "trunc_small"),
            "trunc range",
        )?;
        let converted = self.build(
            self.builder
                .build_float_to_signed_int(value, integer_ty, "trunc integer"),
            "trunc conversion",
        )?;
        let restored = self.build(
            self.builder
                .build_signed_int_to_float(converted, float_ty, "trunc float"),
            "trunc conversion",
        )?;
        let signed_zero = self.build(
            self.builder.build_float_mul(value, zero, "signed zero"),
            "signed zero",
        )?;
        let small = self
            .build(
                self.builder
                    .build_select(below_one, signed_zero, restored, "trunc small"),
                "trunc select",
            )?
            .into_float_value();
        let large = self
            .build(
                self.builder
                    .build_select(already_integer, value, small, "trunc large"),
                "trunc select",
            )?
            .into_float_value();
        Ok(self
            .build(
                self.builder.build_select(is_nan, value, large, "trunc"),
                "trunc select",
            )?
            .into_float_value())
    }

    fn float_rounding(
        &self,
        intrinsic: IntrinsicOp,
        value: FloatValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let truncated = self.float_trunc(value)?;
        let zero = value.get_type().const_float(0.0);
        let one = value.get_type().const_float(1.0);
        let negative = self.build(
            self.builder
                .build_float_compare(FloatPredicate::OLT, value, zero, "negative"),
            "rounding sign",
        )?;
        if intrinsic == IntrinsicOp::FloatTrunc {
            return Ok(truncated.into());
        }
        if intrinsic == IntrinsicOp::FloatFloor || intrinsic == IntrinsicOp::FloatCeil {
            let changed = self.build(
                self.builder
                    .build_float_compare(FloatPredicate::ONE, value, truncated, "changed"),
                "rounding change",
            )?;
            let applies = if intrinsic == IntrinsicOp::FloatFloor {
                self.build(
                    self.builder.build_and(negative, changed, "floor applies"),
                    "rounding condition",
                )?
            } else {
                let positive = self.build(
                    self.builder.build_not(negative, "positive"),
                    "rounding sign",
                )?;
                self.build(
                    self.builder.build_and(positive, changed, "ceil applies"),
                    "rounding condition",
                )?
            };
            let adjusted = if intrinsic == IntrinsicOp::FloatFloor {
                self.build(
                    self.builder.build_float_sub(truncated, one, "floor"),
                    "floor",
                )?
            } else {
                self.build(self.builder.build_float_add(truncated, one, "ceil"), "ceil")?
            };
            return self.build(
                self.builder
                    .build_select(applies, adjusted, truncated, "rounding"),
                "rounding select",
            );
        }

        let difference = self.build(
            self.builder
                .build_float_sub(value, truncated, "round difference"),
            "rounding difference",
        )?;
        let half = value.get_type().const_float(0.5);
        let negative_half = value.get_type().const_float(-0.5);
        let up = self.build(
            self.builder
                .build_float_compare(FloatPredicate::OGE, difference, half, "round up"),
            "rounding comparison",
        )?;
        let down = self.build(
            self.builder.build_float_compare(
                FloatPredicate::OLE,
                difference,
                negative_half,
                "round down",
            ),
            "rounding comparison",
        )?;
        let away = self.build(
            self.builder.build_or(up, down, "round away"),
            "rounding condition",
        )?;
        let delta = self
            .build(
                self.builder.build_select(
                    negative,
                    value.get_type().const_float(-1.0),
                    one,
                    "round delta",
                ),
                "rounding delta",
            )?
            .into_float_value();
        let adjusted = self.build(
            self.builder.build_float_add(truncated, delta, "round"),
            "rounding",
        )?;
        self.build(
            self.builder
                .build_select(away, adjusted, truncated, "round"),
            "rounding select",
        )
    }

    fn float_type_name(
        &self,
        float_ty: inkwell::types::FloatType<'ctx>,
    ) -> Result<&'static str, CodegenError> {
        let width = float_ty.get_bit_width();
        match width {
            32 => Ok("f32"),
            64 => Ok("f64"),
            _ => Err(CodegenError::internal(format!(
                "unsupported floating-point width `{width}`"
            ))),
        }
    }

    pub(crate) fn intrinsic_function(
        &self,
        name: &str,
        parameters: &[BasicMetadataTypeEnum<'ctx>],
        returns: BasicTypeEnum<'ctx>,
    ) -> Result<FunctionValue<'ctx>, CodegenError> {
        if let Some(function) = self.module.get_function(name) {
            return Ok(function);
        }
        let function_type = returns.fn_type(parameters, false);
        Ok(self.module.add_function(name, function_type, None))
    }

    pub(crate) fn void_intrinsic_function(
        &self,
        name: &str,
        parameters: &[BasicMetadataTypeEnum<'ctx>],
    ) -> Result<FunctionValue<'ctx>, CodegenError> {
        if let Some(function) = self.module.get_function(name) {
            return Ok(function);
        }
        let function_type = self.context.void_type().fn_type(parameters, false);
        Ok(self.module.add_function(name, function_type, None))
    }

    fn call_intrinsic(
        &self,
        function: FunctionValue<'ctx>,
        arguments: &[BasicMetadataValueEnum<'ctx>],
    ) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        let call = self.build(
            self.builder.build_call(function, arguments, "intrinsic"),
            "intrinsic call",
        )?;
        match call.try_as_basic_value() {
            inkwell::values::ValueKind::Basic(value) => Ok(value),
            inkwell::values::ValueKind::Instruction(_) => Err(CodegenError::internal(
                "an intrinsic returned no LLVM value",
            )),
        }
    }

    fn i32_type(&self) -> Result<IntType<'ctx>, CodegenError> {
        Ok(self
            .mapper
            .basic(self.tcx.int_ty(seal_ty::IntTy::I32))?
            .into_int_type())
    }

    pub(crate) fn unit_value(&self) -> BasicValueEnum<'ctx> {
        self.mapper.empty_type().const_zero().into()
    }

    fn check_or_panic(
        &self,
        condition: IntValue<'ctx>,
        span: Span,
        message: &str,
    ) -> Result<(), CodegenError> {
        let continue_block = self
            .context
            .append_basic_block(self.function, "intrinsic.continue");
        let panic_block = self
            .context
            .append_basic_block(self.function, "intrinsic.panic");
        self.build(
            self.builder
                .build_conditional_branch(condition, continue_block, panic_block),
            "method contract branch",
        )?;
        self.builder.position_at_end(panic_block);
        self.panic_at(span, message)?;
        self.builder.position_at_end(continue_block);
        Ok(())
    }

    fn bounds_check(
        &self,
        index: IntValue<'ctx>,
        length: IntValue<'ctx>,
        span: Span,
    ) -> Result<(), CodegenError> {
        let in_bounds = self.build(
            self.builder
                .build_int_compare(IntPredicate::ULT, index, length, "index in bounds"),
            "bounds comparison",
        )?;
        self.check_or_panic(in_bounds, span, "index out of bounds")
    }

    fn swap_at(
        &self,
        sequence: &Sequence<'ctx>,
        first: IntValue<'ctx>,
        second: IntValue<'ctx>,
    ) -> Result<(), CodegenError> {
        let element_ty = self.mapper.basic(sequence.element)?;
        let first_ptr = self.index_ptr(sequence.data, first, sequence.element)?;
        let second_ptr = self.index_ptr(sequence.data, second, sequence.element)?;
        let first_value = self.build(
            self.builder
                .build_load(element_ty, first_ptr, "first element"),
            "element load",
        )?;
        let second_value = self.build(
            self.builder
                .build_load(element_ty, second_ptr, "second element"),
            "element load",
        )?;
        self.build(
            self.builder.build_store(first_ptr, second_value),
            "first store",
        )?;
        self.build(
            self.builder.build_store(second_ptr, first_value),
            "second store",
        )?;
        Ok(())
    }

    fn reverse(&self, sequence: &Sequence<'ctx>, span: Span) -> Result<(), CodegenError> {
        let usize = self.mapper.usize_type()?;
        let left_storage = self.entry_alloca(usize.into(), "reverse.left")?;
        let right_storage = self.entry_alloca(usize.into(), "reverse.right")?;
        self.build(
            self.builder.build_store(left_storage, usize.const_zero()),
            "reverse left init",
        )?;
        let last = self.build(
            self.builder
                .build_int_sub(sequence.length, usize.const_int(1, false), "last index"),
            "last index",
        )?;
        // `length - 1` wraps for an empty sequence. Start both ends at zero so
        // the `left < right` condition is false without a separate loop test.
        let empty = self.build(
            self.builder.build_int_compare(
                IntPredicate::EQ,
                sequence.length,
                usize.const_zero(),
                "empty sequence",
            ),
            "empty comparison",
        )?;
        let right = self.build(
            self.builder
                .build_select(empty, usize.const_zero(), last, "right start"),
            "right start",
        )?;
        self.build(
            self.builder.build_store(right_storage, right),
            "reverse right init",
        )?;

        let loop_block = self
            .context
            .append_basic_block(self.function, "reverse.loop");
        let body_block = self
            .context
            .append_basic_block(self.function, "reverse.body");
        let done_block = self
            .context
            .append_basic_block(self.function, "reverse.done");
        self.build(
            self.builder.build_unconditional_branch(loop_block),
            "reverse branch",
        )?;

        self.builder.position_at_end(loop_block);
        let left = self
            .build(
                self.builder.build_load(usize, left_storage, "reverse left"),
                "reverse left load",
            )?
            .into_int_value();
        let right = self
            .build(
                self.builder
                    .build_load(usize, right_storage, "reverse right"),
                "reverse right load",
            )?
            .into_int_value();
        let condition = self.build(
            self.builder
                .build_int_compare(IntPredicate::ULT, left, right, "reverse condition"),
            "reverse comparison",
        )?;
        self.build(
            self.builder
                .build_conditional_branch(condition, body_block, done_block),
            "reverse branch",
        )?;

        self.builder.position_at_end(body_block);
        self.swap_at(sequence, left, right)?;
        let next_left = self.build(
            self.builder
                .build_int_add(left, usize.const_int(1, false), "next left"),
            "reverse increment",
        )?;
        let next_right = self.build(
            self.builder
                .build_int_sub(right, usize.const_int(1, false), "next right"),
            "reverse decrement",
        )?;
        self.build(
            self.builder.build_store(left_storage, next_left),
            "reverse left store",
        )?;
        self.build(
            self.builder.build_store(right_storage, next_right),
            "reverse right store",
        )?;
        self.build(
            self.builder.build_unconditional_branch(loop_block),
            "reverse back edge",
        )?;

        self.builder.position_at_end(done_block);
        let _ = span;
        Ok(())
    }

    fn fill(
        &self,
        sequence: &Sequence<'ctx>,
        value: BasicValueEnum<'ctx>,
    ) -> Result<(), CodegenError> {
        let usize = self.mapper.usize_type()?;
        let index_storage = self.entry_alloca(usize.into(), "fill.index")?;
        self.build(
            self.builder.build_store(index_storage, usize.const_zero()),
            "fill index init",
        )?;
        let loop_block = self.context.append_basic_block(self.function, "fill.loop");
        let body_block = self.context.append_basic_block(self.function, "fill.body");
        let done_block = self.context.append_basic_block(self.function, "fill.done");
        self.build(
            self.builder.build_unconditional_branch(loop_block),
            "fill branch",
        )?;

        self.builder.position_at_end(loop_block);
        let index = self
            .build(
                self.builder.build_load(usize, index_storage, "fill index"),
                "fill index load",
            )?
            .into_int_value();
        let condition = self.build(
            self.builder.build_int_compare(
                IntPredicate::ULT,
                index,
                sequence.length,
                "fill condition",
            ),
            "fill comparison",
        )?;
        self.build(
            self.builder
                .build_conditional_branch(condition, body_block, done_block),
            "fill branch",
        )?;

        self.builder.position_at_end(body_block);
        let element = self.index_ptr(sequence.data, index, sequence.element)?;
        self.build(self.builder.build_store(element, value), "fill store")?;
        let next = self.build(
            self.builder
                .build_int_add(index, usize.const_int(1, false), "next index"),
            "fill increment",
        )?;
        self.build(
            self.builder.build_store(index_storage, next),
            "fill index store",
        )?;
        self.build(
            self.builder.build_unconditional_branch(loop_block),
            "fill back edge",
        )?;

        self.builder.position_at_end(done_block);
        Ok(())
    }
}
