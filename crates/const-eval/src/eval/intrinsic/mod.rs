//! The closed intrinsic whitelist for constant evaluation.
//!
//! Every operation mirrors `seal-codegen`; a difference here would make a
//! compile-time result disagree with the same method at run time.

use seal_methods::{IntrinsicOp, MethodId, MethodOwner};
use seal_mir::{IntrinsicReceiver, Operand};
use seal_span::Span;
use seal_ty::{TyId, TyKind};

use super::*;

mod float;
mod integer;
mod raw_pointer;
mod sequence;

impl<'a> Interpreter<'a> {
    pub(super) fn eval_intrinsic(
        &mut self,
        method: MethodId,
        receiver: &IntrinsicReceiver,
        args: &[Operand],
        result_ty: TyId,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        let definition = method.def();
        match definition.owner {
            MethodOwner::Array | MethodOwner::Slice => {
                self.sequence_intrinsic(definition.intrinsic, receiver, args, result_ty, span)
            }
            MethodOwner::RawPtr => {
                self.raw_pointer_intrinsic(definition.intrinsic, receiver, args, result_ty, span)
            }
            _ => self.scalar_intrinsic(
                definition.owner,
                definition.intrinsic,
                receiver,
                args,
                result_ty,
                span,
            ),
        }
    }

    fn scalar_intrinsic(
        &mut self,
        owner: MethodOwner,
        intrinsic: IntrinsicOp,
        receiver: &IntrinsicReceiver,
        args: &[Operand],
        result_ty: TyId,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        if intrinsic == IntrinsicOp::BoolToggle {
            let IntrinsicReceiver::Place(place) = receiver else {
                return Err(self.error(
                    EvalErrorKind::Internal("`bool::toggle` has a non-place receiver".into()),
                    span,
                ));
            };
            let mut resolved = self.resolve_place(place, span)?;
            let value = self.read_place_raw(&resolved, span)?;
            let ConstValue::Bool(value) = value else {
                return Err(self.error(
                    EvalErrorKind::Internal("`bool::toggle` receiver is not a bool".into()),
                    span,
                ));
            };
            resolved.slice = None;
            returned_unit(self.write_place(&resolved, ConstValue::Bool(!value), span))
        } else {
            let value = self.receiver_value(receiver, span)?;
            self.scalar_operation(owner, intrinsic, &value, args, result_ty, span)
        }
    }

    fn scalar_operation(
        &mut self,
        owner: MethodOwner,
        intrinsic: IntrinsicOp,
        value: &ConstValue,
        args: &[Operand],
        result_ty: TyId,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        match intrinsic {
            IntrinsicOp::IntAbs
            | IntrinsicOp::IntSignum
            | IntrinsicOp::IntMin
            | IntrinsicOp::IntMax
            | IntrinsicOp::IntClamp
            | IntrinsicOp::IntWrappingAdd
            | IntrinsicOp::IntWrappingSub
            | IntrinsicOp::IntWrappingMul
            | IntrinsicOp::IntSaturatingAdd
            | IntrinsicOp::IntSaturatingSub
            | IntrinsicOp::IntSaturatingMul
            | IntrinsicOp::IntCountOnes
            | IntrinsicOp::IntLeadingZeros
            | IntrinsicOp::IntTrailingZeros
            | IntrinsicOp::IntRotateLeft
            | IntrinsicOp::IntRotateRight
            | IntrinsicOp::IntSwapBytes => {
                self.integer_intrinsic(owner, intrinsic, value, args, result_ty, span)
            }
            IntrinsicOp::FloatAbs
            | IntrinsicOp::FloatMin
            | IntrinsicOp::FloatMax
            | IntrinsicOp::FloatClamp
            | IntrinsicOp::FloatIsNan
            | IntrinsicOp::FloatIsFinite
            | IntrinsicOp::FloatIsInfinite
            | IntrinsicOp::FloatSqrt
            | IntrinsicOp::FloatFloor
            | IntrinsicOp::FloatCeil
            | IntrinsicOp::FloatRound
            | IntrinsicOp::FloatTrunc
            | IntrinsicOp::FloatFract
            | IntrinsicOp::FloatPowi => {
                self.float_intrinsic(intrinsic, value, args, result_ty, span)
            }
            IntrinsicOp::BoolNot | IntrinsicOp::BoolToI32 => {
                let ConstValue::Bool(value) = value else {
                    return Err(self.error(
                        EvalErrorKind::Internal("bool method received a non-bool".into()),
                        span,
                    ));
                };
                let value = *value;
                match intrinsic {
                    IntrinsicOp::BoolNot => Ok(ConstValue::Bool(!value)),
                    _ => Ok(ConstValue::Int {
                        bits: u128::from(value),
                        ty: result_ty,
                    }),
                }
            }
            IntrinsicOp::CharIsAscii
            | IntrinsicOp::CharIsAsciiAlphabetic
            | IntrinsicOp::CharIsAsciiDigit
            | IntrinsicOp::CharIsAsciiWhitespace
            | IntrinsicOp::CharToAsciiUppercase
            | IntrinsicOp::CharToAsciiLowercase
            | IntrinsicOp::CharLenUtf8
            | IntrinsicOp::CharToU32 => {
                let ConstValue::Char(value) = value else {
                    return Err(self.error(
                        EvalErrorKind::Internal("char method received a non-char".into()),
                        span,
                    ));
                };
                let value = *value;
                let result = match intrinsic {
                    IntrinsicOp::CharIsAscii => ConstValue::Bool(value.is_ascii()),
                    IntrinsicOp::CharIsAsciiAlphabetic => {
                        ConstValue::Bool(value.is_ascii_alphabetic())
                    }
                    IntrinsicOp::CharIsAsciiDigit => ConstValue::Bool(value.is_ascii_digit()),
                    IntrinsicOp::CharIsAsciiWhitespace => {
                        ConstValue::Bool(value.is_ascii_whitespace())
                    }
                    IntrinsicOp::CharToAsciiUppercase => {
                        ConstValue::Char(value.to_ascii_uppercase())
                    }
                    IntrinsicOp::CharToAsciiLowercase => {
                        ConstValue::Char(value.to_ascii_lowercase())
                    }
                    IntrinsicOp::CharLenUtf8 => ConstValue::Int {
                        bits: u128::from(value.len_utf8() as u64),
                        ty: result_ty,
                    },
                    _ => ConstValue::Int {
                        bits: u128::from(value as u32),
                        ty: result_ty,
                    },
                };
                Ok(result)
            }
            IntrinsicOp::BoolToggle
            | IntrinsicOp::Len
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
            | IntrinsicOp::PtrRead
            | IntrinsicOp::PtrWrite
            | IntrinsicOp::PtrReadUnaligned
            | IntrinsicOp::PtrWriteUnaligned
            | IntrinsicOp::PtrCopy
            | IntrinsicOp::PtrCopyNonOverlapping => Err(self.error(
                EvalErrorKind::Internal(format!(
                    "intrinsic `{intrinsic:?}` reached the wrong path"
                )),
                span,
            )),
        }
    }

    pub(super) fn receiver_value(
        &mut self,
        receiver: &IntrinsicReceiver,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        match receiver {
            IntrinsicReceiver::Place(place) => {
                let resolved = self.resolve_place(place, span)?;
                self.read_place_raw(&resolved, span)
            }
            IntrinsicReceiver::Value(operand) => self.read_operand(operand, span),
        }
    }

    pub(super) fn int_argument(
        &mut self,
        operand: &Operand,
        span: Span,
    ) -> Result<u128, EvalError> {
        let value = self.read_operand(operand, span)?;
        self.as_u128(&value, span)
    }

    pub(super) fn float_argument(
        &mut self,
        operand: &Operand,
        span: Span,
    ) -> Result<f64, EvalError> {
        let value = self.read_operand(operand, span)?;
        match value {
            ConstValue::Float { bits, ty } => {
                let width = self.float_width(ty, span)?;
                Ok(ops::float_value(bits, width))
            }
            _ => Err(self.error(
                EvalErrorKind::Internal("float argument is not a float".into()),
                span,
            )),
        }
    }
}

fn returned_unit(result: Result<(), EvalError>) -> Result<ConstValue, EvalError> {
    result.map(|()| ConstValue::Unit)
}
