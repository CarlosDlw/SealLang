//! Scalar semantics shared by const evaluation and runtime code generation.
//!
//! The rules here mirror `seal-codegen`: wrapping integer arithmetic, masked
//! shift amounts, masked truncation on casts, and saturating float-to-integer
//! conversion. A mismatch would make a constant disagree with the same
//! expression at run time.

use seal_mir::{BinOp, UnOp};
use seal_ty::{NumericCastKind, TyId};

use crate::error::EvalErrorKind;
use crate::value::ConstValue;

/// Truncates `value` to `bits` significant bits.
pub(crate) const fn truncate(value: u128, bits: u32) -> u128 {
    if bits >= 128 {
        value
    } else {
        value & ((1u128 << bits) - 1)
    }
}

/// Reinterprets the low `bits` of `value` as a signed number.
pub(crate) const fn sign_extend(value: u128, bits: u32) -> i128 {
    if bits >= 128 {
        value as i128
    } else {
        let shift = 128 - bits;
        ((value << shift) as i128) >> shift
    }
}

/// Truncates `value` to `bits` and keeps it as two's complement bits.
pub(crate) const fn normalize_signed(value: i128, bits: u32) -> u128 {
    truncate(value as u128, bits)
}

/// Returns the most negative signed value of `bits` width.
const fn signed_min(bits: u32) -> i128 {
    if bits >= 128 {
        i128::MIN
    } else {
        -(1i128 << (bits - 1))
    }
}

/// Applies a binary operator to two integers.
pub(crate) fn int_binary(
    op: BinOp,
    lhs: u128,
    rhs: u128,
    bits: u32,
    signed: bool,
    ty: TyId,
) -> Result<ConstValue, EvalErrorKind> {
    let bits_out = match op {
        BinOp::Add => truncate(lhs.wrapping_add(rhs), bits),
        BinOp::Sub => truncate(lhs.wrapping_sub(rhs), bits),
        BinOp::Mul => truncate(lhs.wrapping_mul(rhs), bits),
        BinOp::Div => {
            if rhs == 0 {
                return Err(EvalErrorKind::DivisionByZero);
            }
            if signed {
                let lhs = sign_extend(lhs, bits);
                let rhs = sign_extend(rhs, bits);
                if lhs == signed_min(bits) && rhs == -1 {
                    return Err(EvalErrorKind::SignedOverflow);
                }
                normalize_signed(lhs / rhs, bits)
            } else {
                truncate(lhs / rhs, bits)
            }
        }
        BinOp::Rem => {
            if rhs == 0 {
                return Err(EvalErrorKind::RemainderByZero);
            }
            if signed {
                let lhs = sign_extend(lhs, bits);
                let rhs = sign_extend(rhs, bits);
                if lhs == signed_min(bits) && rhs == -1 {
                    normalize_signed(0, bits)
                } else {
                    normalize_signed(lhs % rhs, bits)
                }
            } else {
                truncate(lhs % rhs, bits)
            }
        }
        BinOp::BitAnd => truncate(lhs & rhs, bits),
        BinOp::BitOr => truncate(lhs | rhs, bits),
        BinOp::BitXor => truncate(lhs ^ rhs, bits),
        BinOp::Shl => {
            let shift = (rhs & u128::from(bits.saturating_sub(1))) as u32;
            truncate(lhs << shift, bits)
        }
        BinOp::Shr => {
            let shift = (rhs & u128::from(bits.saturating_sub(1))) as u32;
            if signed {
                normalize_signed(sign_extend(lhs, bits) >> shift, bits)
            } else {
                truncate(lhs >> shift, bits)
            }
        }
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
            unreachable!("comparisons produce booleans, not integers")
        }
    };
    Ok(ConstValue::Int { bits: bits_out, ty })
}

/// Compares two integers with the operator's signedness.
pub(crate) fn int_compare(op: BinOp, lhs: u128, rhs: u128, bits: u32, signed: bool) -> bool {
    let equal = lhs == rhs;
    if signed {
        let lhs = sign_extend(lhs, bits);
        let rhs = sign_extend(rhs, bits);
        match op {
            BinOp::Eq => equal,
            BinOp::Ne => !equal,
            BinOp::Lt => lhs < rhs,
            BinOp::Gt => lhs > rhs,
            BinOp::Le => lhs <= rhs,
            BinOp::Ge => lhs >= rhs,
            _ => unreachable!("not an integer comparison"),
        }
    } else {
        match op {
            BinOp::Eq => equal,
            BinOp::Ne => !equal,
            BinOp::Lt => lhs < rhs,
            BinOp::Gt => lhs > rhs,
            BinOp::Le => lhs <= rhs,
            BinOp::Ge => lhs >= rhs,
            _ => unreachable!("not an integer comparison"),
        }
    }
}

/// Returns the decoded floating-point value.
pub(crate) fn float_value(bits: u64, width: u32) -> f64 {
    if width == 32 {
        f64::from(f32::from_bits(bits as u32))
    } else {
        f64::from_bits(bits)
    }
}

/// Encodes a floating-point value at the requested width.
pub(crate) fn float_bits(value: f64, width: u32) -> u64 {
    if width == 32 {
        u64::from((value as f32).to_bits())
    } else {
        value.to_bits()
    }
}

/// Applies a binary operator to two floating-point values.
pub(crate) fn float_binary(
    op: BinOp,
    lhs: u64,
    rhs: u64,
    width: u32,
    ty: TyId,
) -> Result<ConstValue, EvalErrorKind> {
    let lhs = float_value(lhs, width);
    let rhs = float_value(rhs, width);
    let bits = match op {
        BinOp::Add => float_bits(lhs + rhs, width),
        BinOp::Sub => float_bits(lhs - rhs, width),
        BinOp::Mul => float_bits(lhs * rhs, width),
        BinOp::Div => float_bits(lhs / rhs, width),
        BinOp::Rem => float_bits(lhs % rhs, width),
        BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
            return Ok(ConstValue::Bool(float_compare(op, lhs, rhs)));
        }
        _ => {
            return Err(EvalErrorKind::Internal(format!(
                "operator `{}` is not defined for floats",
                op.as_str()
            )));
        }
    };
    Ok(ConstValue::Float { bits, ty })
}

/// Compares two floating-point values, IEEE style.
pub(crate) fn float_compare(op: BinOp, lhs: f64, rhs: f64) -> bool {
    match op {
        BinOp::Eq => lhs == rhs,
        BinOp::Ne => lhs != rhs,
        BinOp::Lt => lhs < rhs,
        BinOp::Gt => lhs > rhs,
        BinOp::Le => lhs <= rhs,
        BinOp::Ge => lhs >= rhs,
        _ => unreachable!("not a floating-point comparison"),
    }
}

/// Applies a unary operator to an integer.
pub(crate) fn int_unary(
    op: UnOp,
    value: u128,
    bits: u32,
    ty: TyId,
) -> Result<ConstValue, EvalErrorKind> {
    let bits = match op {
        UnOp::Neg => truncate(0u128.wrapping_sub(value), bits),
        UnOp::BitNot => truncate(!value, bits),
        UnOp::Not => {
            return Err(EvalErrorKind::Internal(
                "logical negation requires a boolean".to_owned(),
            ));
        }
    };
    Ok(ConstValue::Int { bits, ty })
}

/// Converts a value between numeric types.
pub(crate) fn cast(
    kind: NumericCastKind,
    value: &ConstValue,
    from_bits: u32,
    from_signed: bool,
    to_bits: u32,
    to_signed: bool,
    to_ty: TyId,
) -> Result<ConstValue, EvalErrorKind> {
    match (kind, value) {
        (
            NumericCastKind::IntToInt,
            ConstValue::Int {
                bits: value_bits, ..
            },
        ) => {
            let extended = if from_signed {
                normalize_signed(sign_extend(*value_bits, from_bits), to_bits)
            } else {
                truncate(*value_bits, to_bits)
            };
            Ok(ConstValue::Int {
                bits: extended,
                ty: to_ty,
            })
        }
        (
            NumericCastKind::IntToFloat,
            ConstValue::Int {
                bits: value_bits, ..
            },
        ) => {
            let value = if from_signed {
                sign_extend(*value_bits, from_bits) as f64
            } else {
                *value_bits as f64
            };
            Ok(ConstValue::Float {
                bits: float_bits(value, to_bits),
                ty: to_ty,
            })
        }
        (NumericCastKind::FloatToInt, ConstValue::Float { bits, .. }) => {
            let value = float_value(*bits, from_bits);
            let converted: u128 = if value.is_nan() {
                0
            } else {
                let truncated = value.trunc();
                if to_signed {
                    if to_bits >= 128 {
                        truncated as i128 as u128
                    } else {
                        let limit = 1i128 << (to_bits - 1);
                        truncated.clamp(-(limit as f64), (limit - 1) as f64) as i128 as u128
                    }
                } else if to_bits >= 128 {
                    truncated as u128
                } else {
                    let max = ((1u128 << to_bits) - 1) as f64;
                    truncated.clamp(0.0, max) as u128
                }
            };
            Ok(ConstValue::Int {
                bits: truncate(converted, to_bits),
                ty: to_ty,
            })
        }
        (NumericCastKind::FloatToFloat, ConstValue::Float { bits, .. }) => {
            let value = float_value(*bits, from_bits);
            Ok(ConstValue::Float {
                bits: float_bits(value, to_bits),
                ty: to_ty,
            })
        }
        (kind, value) => Err(EvalErrorKind::Internal(format!(
            "cast {kind:?} received an unexpected value {value:?}"
        ))),
    }
}
