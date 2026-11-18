//! Floating-point method semantics.

use super::*;

impl<'a> Interpreter<'a> {
    pub(super) fn float_intrinsic(
        &mut self,
        intrinsic: IntrinsicOp,
        value: &ConstValue,
        args: &[Operand],
        result_ty: TyId,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        let ConstValue::Float { bits, ty } = value else {
            return Err(self.error(
                EvalErrorKind::Internal("float method received a non-float".into()),
                span,
            ));
        };
        let width = self.float_width(*ty, span)?;
        let value = ops::float_value(*bits, width);
        let encode = |value: f64| ConstValue::Float {
            bits: ops::float_bits(value, width),
            ty: result_ty,
        };
        let result = match intrinsic {
            IntrinsicOp::FloatAbs => encode(value.abs()),
            IntrinsicOp::FloatSqrt => encode(value.sqrt()),
            IntrinsicOp::FloatFloor => encode(value.floor()),
            IntrinsicOp::FloatCeil => encode(value.ceil()),
            IntrinsicOp::FloatRound => encode(value.round()),
            IntrinsicOp::FloatTrunc => encode(value.trunc()),
            IntrinsicOp::FloatFract => encode(value - value.trunc()),
            IntrinsicOp::FloatIsNan => ConstValue::Bool(value.is_nan()),
            IntrinsicOp::FloatIsFinite => ConstValue::Bool(value.is_finite()),
            IntrinsicOp::FloatIsInfinite => ConstValue::Bool(value.is_infinite()),
            IntrinsicOp::FloatMin | IntrinsicOp::FloatMax => {
                let other = self.float_argument(&args[0], span)?;
                let chosen = if value.is_nan() {
                    other
                } else if other.is_nan()
                    || (intrinsic == IntrinsicOp::FloatMin && value < other)
                    || (intrinsic == IntrinsicOp::FloatMax && value > other)
                {
                    value
                } else {
                    other
                };
                encode(chosen)
            }
            IntrinsicOp::FloatClamp => {
                let minimum = self.float_argument(&args[0], span)?;
                let maximum = self.float_argument(&args[1], span)?;
                if minimum.is_nan() || maximum.is_nan() || minimum > maximum {
                    return Err(self.error(
                        EvalErrorKind::Panic("`clamp` requires minimum <= maximum".to_owned()),
                        span,
                    ));
                }
                let clamped = if value < minimum {
                    minimum
                } else if value > maximum {
                    maximum
                } else {
                    value
                };
                encode(clamped)
            }
            IntrinsicOp::FloatPowi => {
                let exponent = self.int_argument(&args[0], span)?;
                let exponent = exponent as u32 as i32;
                if width == 32 {
                    encode(f64::from((value as f32).powi(exponent)))
                } else {
                    encode(value.powi(exponent))
                }
            }
            _ => {
                return Err(self.error(
                    EvalErrorKind::Internal("non-float intrinsic in the float path".into()),
                    span,
                ));
            }
        };
        Ok(result)
    }
}
