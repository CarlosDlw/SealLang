//! Integer method semantics.

use super::*;

impl<'a> Interpreter<'a> {
    pub(super) fn integer_intrinsic(
        &mut self,
        owner: MethodOwner,
        intrinsic: IntrinsicOp,
        value: &ConstValue,
        args: &[Operand],
        result_ty: TyId,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        let ConstValue::Int { bits, ty } = value else {
            return Err(self.error(
                EvalErrorKind::Internal("integer method received a non-integer".into()),
                span,
            ));
        };
        let (width, signed) = self.integer_info(*ty, span)?;
        let signed = signed && owner == MethodOwner::Int;
        let value = ops::truncate(*bits, width);
        let result = match intrinsic {
            IntrinsicOp::IntAbs => {
                let value = ops::sign_extend(value, width);
                ops::truncate(value.wrapping_abs() as u128, width)
            }
            IntrinsicOp::IntSignum => {
                if signed {
                    let value = ops::sign_extend(value, width);
                    ops::normalize_signed(value.signum(), width)
                } else {
                    u128::from(value != 0)
                }
            }
            IntrinsicOp::IntMin | IntrinsicOp::IntMax => {
                let other = self.int_argument(&args[0], span)?;
                let less = if signed {
                    ops::sign_extend(value, width) < ops::sign_extend(other, width)
                } else {
                    value < other
                };
                if (intrinsic == IntrinsicOp::IntMin) == less {
                    value
                } else {
                    ops::truncate(other, width)
                }
            }
            IntrinsicOp::IntClamp => {
                let minimum = self.int_argument(&args[0], span)?;
                let maximum = self.int_argument(&args[1], span)?;
                let ordered = if signed {
                    ops::sign_extend(minimum, width) <= ops::sign_extend(maximum, width)
                } else {
                    minimum <= maximum
                };
                if !ordered {
                    return Err(self.error(
                        EvalErrorKind::Panic("`clamp` requires minimum <= maximum".to_owned()),
                        span,
                    ));
                }
                let below = if signed {
                    ops::sign_extend(value, width) < ops::sign_extend(minimum, width)
                } else {
                    value < minimum
                };
                let above = if signed {
                    ops::sign_extend(value, width) > ops::sign_extend(maximum, width)
                } else {
                    value > maximum
                };
                if below {
                    ops::truncate(minimum, width)
                } else if above {
                    ops::truncate(maximum, width)
                } else {
                    value
                }
            }
            IntrinsicOp::IntWrappingAdd
            | IntrinsicOp::IntWrappingSub
            | IntrinsicOp::IntWrappingMul => {
                let other = self.int_argument(&args[0], span)?;
                let result = match intrinsic {
                    IntrinsicOp::IntWrappingAdd => value.wrapping_add(other),
                    IntrinsicOp::IntWrappingSub => value.wrapping_sub(other),
                    _ => value.wrapping_mul(other),
                };
                ops::truncate(result, width)
            }
            IntrinsicOp::IntSaturatingAdd
            | IntrinsicOp::IntSaturatingSub
            | IntrinsicOp::IntSaturatingMul => {
                let other = self.int_argument(&args[0], span)?;
                self.saturating(intrinsic, value, other, width, signed, span)?
            }
            IntrinsicOp::IntCountOnes => u128::from(value.count_ones()),
            IntrinsicOp::IntLeadingZeros => u128::from(if value == 0 {
                width
            } else {
                value.leading_zeros() - (128 - width)
            }),
            IntrinsicOp::IntTrailingZeros => u128::from(if value == 0 {
                width
            } else {
                value.trailing_zeros()
            }),
            IntrinsicOp::IntRotateLeft | IntrinsicOp::IntRotateRight => {
                let amount = self.int_argument(&args[0], span)? as u32 % width;
                if width >= 128 {
                    if intrinsic == IntrinsicOp::IntRotateLeft {
                        value.rotate_left(amount)
                    } else {
                        value.rotate_right(amount)
                    }
                } else if amount == 0 {
                    value
                } else if intrinsic == IntrinsicOp::IntRotateLeft {
                    ops::truncate((value << amount) | (value >> (width - amount)), width)
                } else {
                    ops::truncate((value >> amount) | (value << (width - amount)), width)
                }
            }
            IntrinsicOp::IntSwapBytes => {
                if width <= 8 {
                    value
                } else {
                    let bytes = (width / 8) as usize;
                    let mut swapped = 0u128;
                    for index in 0..bytes {
                        swapped = (swapped << 8) | ((value >> (index * 8)) & 0xff);
                    }
                    swapped
                }
            }
            _ => {
                return Err(self.error(
                    EvalErrorKind::Internal("non-integer intrinsic in the integer path".into()),
                    span,
                ));
            }
        };
        let ty = if matches!(
            intrinsic,
            IntrinsicOp::IntCountOnes
                | IntrinsicOp::IntLeadingZeros
                | IntrinsicOp::IntTrailingZeros
        ) {
            result_ty
        } else {
            *ty
        };
        Ok(ConstValue::Int { bits: result, ty })
    }

    fn saturating(
        &self,
        intrinsic: IntrinsicOp,
        lhs: u128,
        rhs: u128,
        width: u32,
        signed: bool,
        span: Span,
    ) -> Result<u128, EvalError> {
        if signed {
            let minimum = if width >= 128 {
                i128::MIN
            } else {
                -(1i128 << (width - 1))
            };
            let maximum = if width >= 128 {
                i128::MAX
            } else {
                (1i128 << (width - 1)) - 1
            };
            let lhs = ops::sign_extend(lhs, width);
            let rhs = ops::sign_extend(rhs, width);
            let result = match intrinsic {
                IntrinsicOp::IntSaturatingAdd => lhs.saturating_add(rhs),
                IntrinsicOp::IntSaturatingSub => lhs.saturating_sub(rhs),
                _ => lhs.saturating_mul(rhs),
            };
            if result < minimum {
                Ok(ops::normalize_signed(minimum, width))
            } else if result > maximum {
                Ok(ops::normalize_signed(maximum, width))
            } else {
                Ok(ops::normalize_signed(result, width))
            }
        } else {
            let maximum = if width >= 128 {
                u128::MAX
            } else {
                (1u128 << width) - 1
            };
            let result = match intrinsic {
                IntrinsicOp::IntSaturatingAdd => lhs.saturating_add(rhs),
                IntrinsicOp::IntSaturatingSub => lhs.saturating_sub(rhs),
                _ => lhs.saturating_mul(rhs),
            };
            let _ = span;
            Ok(result.min(maximum))
        }
    }
}
