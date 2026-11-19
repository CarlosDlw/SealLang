//! Raw pointer method semantics during constant evaluation.
//!
//! A constant pointer is represented by the same value as the reference it came
//! from: `Ref`, `Slice`, `Str`, `Dyn`, or an integer for a pointer built from an
//! address. The operations below preserve that representation; only `data()`
//! converts a slice to its thin element pointer.

use seal_methods::IntrinsicOp;
use seal_mir::{IntrinsicReceiver, Operand};
use seal_span::Span;
use seal_ty::{RawPtrKind, RefKind, TyId, TyKind};

use super::*;
use crate::ops;
use crate::place::{self, PathStep, ResolvedPlace};

impl<'a> Interpreter<'a> {
    pub(super) fn raw_pointer_intrinsic(
        &mut self,
        intrinsic: IntrinsicOp,
        receiver: &IntrinsicReceiver,
        args: &[Operand],
        result_ty: TyId,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        let value = self.receiver_value(receiver, span)?;
        match intrinsic {
            IntrinsicOp::PtrIsNull => Ok(ConstValue::Bool(raw_is_null(&value))),
            IntrinsicOp::PtrLen => self.raw_length(&value, result_ty, span),
            IntrinsicOp::PtrData => self.raw_data(value, result_ty, span),
            IntrinsicOp::PtrAddr | IntrinsicOp::PtrExposeAddr => {
                Err(self.error(EvalErrorKind::AddressNotComputable, span))
            }
            IntrinsicOp::PtrWithAddr => {
                let address = self.int_argument(&args[0], span)?;
                match value {
                    ConstValue::Int { .. } => Ok(ConstValue::Int {
                        bits: ops::truncate(address, self.pointer_bits(result_ty)?),
                        ty: result_ty,
                    }),
                    _ => Err(self.error(EvalErrorKind::RawPointerAccess, span)),
                }
            }
            IntrinsicOp::PtrAdd
            | IntrinsicOp::PtrSub
            | IntrinsicOp::PtrOffset
            | IntrinsicOp::PtrByteAdd
            | IntrinsicOp::PtrByteOffset
            | IntrinsicOp::PtrWrappingAdd
            | IntrinsicOp::PtrWrappingSub
            | IntrinsicOp::PtrWrappingOffset => {
                self.raw_offset(intrinsic, value, &args[0], result_ty, span)
            }
            IntrinsicOp::PtrOffsetFrom => {
                let other = self.read_operand(&args[0], span)?;
                self.raw_offset_from(value, other, result_ty, span)
            }
            IntrinsicOp::PtrRead | IntrinsicOp::PtrReadUnaligned => self.raw_read(value, span),
            IntrinsicOp::PtrWrite
            | IntrinsicOp::PtrWriteUnaligned
            | IntrinsicOp::PtrCopy
            | IntrinsicOp::PtrCopyNonOverlapping => {
                Err(self.error(EvalErrorKind::RawPointerWrite, span))
            }
            _ => Err(self.error(
                EvalErrorKind::Internal(
                    "a non-pointer intrinsic reached raw pointer evaluation".into(),
                ),
                span,
            )),
        }
    }

    fn raw_length(
        &self,
        value: &ConstValue,
        result_ty: TyId,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        let length = match value {
            ConstValue::Slice { length, .. } => *length,
            ConstValue::Str { value, .. } => self.interner.resolve(*value).len() as u64,
            _ => return Err(self.error(EvalErrorKind::RawPointerAccess, span)),
        };
        Ok(ConstValue::Int {
            bits: u128::from(length),
            ty: result_ty,
        })
    }

    fn raw_data(
        &self,
        value: ConstValue,
        result_ty: TyId,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        match value {
            ConstValue::Slice {
                target,
                offset,
                element,
                ..
            } => Ok(ConstValue::Ref {
                target,
                offset,
                pointee: element,
                kind: self.raw_ref_kind(result_ty),
            }),
            // A thin pointer has no metadata to discard.
            ConstValue::Ref { .. } | ConstValue::Int { .. } => Ok(value),
            ConstValue::Str { .. } | ConstValue::Dyn { .. } => {
                Err(self.error(EvalErrorKind::RawPointerAccess, span))
            }
            _ => Err(self.error(EvalErrorKind::RawPointerAccess, span)),
        }
    }

    fn raw_ref_kind(&self, ty: TyId) -> RefKind {
        match self.tcx.kind(ty) {
            TyKind::RawPtr {
                kind: RawPtrKind::Mut,
                ..
            } => RefKind::Exclusive,
            _ => RefKind::Shared,
        }
    }

    fn raw_offset(
        &mut self,
        intrinsic: IntrinsicOp,
        value: ConstValue,
        operand: &Operand,
        result_ty: TyId,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        let amount = self.pointer_count(operand, span)?;
        let amount = if matches!(intrinsic, IntrinsicOp::PtrSub | IntrinsicOp::PtrWrappingSub) {
            -amount
        } else {
            amount
        };
        let bytewise = matches!(
            intrinsic,
            IntrinsicOp::PtrByteAdd | IntrinsicOp::PtrByteOffset
        );
        let in_bounds = matches!(
            intrinsic,
            IntrinsicOp::PtrAdd
                | IntrinsicOp::PtrSub
                | IntrinsicOp::PtrOffset
                | IntrinsicOp::PtrByteAdd
                | IntrinsicOp::PtrByteOffset
        );
        let (_, pointee) = self.tcx.raw_parts(result_ty).ok_or_else(|| {
            self.error(
                EvalErrorKind::Internal("pointer result is not a raw pointer".into()),
                span,
            )
        })?;
        let size = if bytewise {
            1
        } else {
            self.layout
                .layout_of(pointee)
                .map_err(|error| self.error(EvalErrorKind::Internal(error.to_string()), span))?
                .size()
        };
        let delta = amount.wrapping_mul(i128::from(size));
        match value {
            ConstValue::Int { bits, .. } => {
                let bits = (bits as i128).wrapping_add(delta) as u128;
                Ok(ConstValue::Int {
                    bits: ops::truncate(bits, self.pointer_bits(result_ty)?),
                    ty: result_ty,
                })
            }
            ConstValue::Ref {
                target,
                offset,
                pointee,
                kind,
            } => {
                let next = i128::from(offset).wrapping_add(delta);
                if in_bounds && (next < 0 || next > i128::from(self.memory[target].size)) {
                    return Err(self.error(EvalErrorKind::RawPointerAccess, span));
                }
                Ok(ConstValue::Ref {
                    target,
                    offset: next as u64,
                    pointee,
                    kind,
                })
            }
            _ => Err(self.error(EvalErrorKind::RawPointerAccess, span)),
        }
    }

    fn raw_offset_from(
        &mut self,
        value: ConstValue,
        other: ConstValue,
        result_ty: TyId,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        // Only pointers into the same constant allocation have a distance the
        // evaluator can prove. Integer-built pointers have no allocation.
        let (
            ConstValue::Ref {
                target: left_target,
                offset: left,
                pointee,
                ..
            },
            ConstValue::Ref {
                target: right_target,
                offset: right,
                ..
            },
        ) = (&value, &other)
        else {
            return Err(self.error(EvalErrorKind::RawPointerAccess, span));
        };
        if left_target != right_target {
            return Err(self.error(EvalErrorKind::RawPointerAccess, span));
        }
        let size = self
            .layout
            .layout_of(*pointee)
            .map_err(|error| self.error(EvalErrorKind::Internal(error.to_string()), span))?
            .size();
        let difference = *left as i128 - *right as i128;
        let distance = if size == 0 {
            0
        } else {
            difference / i128::from(size)
        };
        Ok(ConstValue::Int {
            bits: ops::truncate(distance as u128, self.pointer_bits(result_ty)?),
            ty: result_ty,
        })
    }

    fn raw_read(&mut self, value: ConstValue, span: Span) -> Result<ConstValue, EvalError> {
        let ConstValue::Ref {
            target,
            offset,
            pointee,
            ..
        } = value
        else {
            return Err(self.error(EvalErrorKind::RawPointerAccess, span));
        };
        let mut resolved = ResolvedPlace::at(target, pointee);
        resolved.offset = offset;
        resolved.path = self.view_path(target, offset, pointee, span)?;
        self.read_place(&resolved, span)
    }

    /// Locates the value a typed pointer views inside its allocation.
    ///
    /// Offset zero normally names the allocation root, but a pointer to an
    /// element or field that happens to start at offset zero names that member
    /// instead. Comparing the allocation type with the pointee distinguishes
    /// the two cases.
    fn view_path(
        &self,
        target: AllocId,
        offset: u64,
        pointee: TyId,
        span: Span,
    ) -> Result<Vec<PathStep>, EvalError> {
        let allocation = &self.memory[target];
        let root = allocation.value.as_ref().ok_or_else(|| {
            self.error(
                EvalErrorKind::Internal("read of an uninitialized allocation".into()),
                span,
            )
        })?;
        let path = if offset == 0 && allocation.ty != pointee {
            place::locate_member(self.tcx, &self.layout, root, allocation.ty, offset)
        } else {
            place::locate(self.tcx, &self.layout, root, allocation.ty, offset)
        };
        path.ok_or_else(|| {
            self.error(
                EvalErrorKind::Internal(format!(
                    "no value starts at byte offset {offset} in an allocation"
                )),
                span,
            )
        })
    }

    fn pointer_count(&mut self, operand: &Operand, span: Span) -> Result<i128, EvalError> {
        let value = self.read_operand(operand, span)?;
        let ConstValue::Int { bits, ty } = value else {
            return Err(self.error(EvalErrorKind::RawPointerAccess, span));
        };
        let (width, signed) = self.integer_info(ty, span)?;
        Ok(if signed {
            let shift = 128 - width;
            ((bits << shift) as i128) >> shift
        } else {
            bits as i128
        })
    }

    fn pointer_bits(&self, ty: TyId) -> Result<u32, EvalError> {
        self.layout
            .layout_of(ty)
            .map(|layout| (layout.size() * 8) as u32)
            .map_err(|error| {
                self.error(
                    EvalErrorKind::Internal(error.to_string()),
                    seal_span::Span::default(),
                )
            })
    }
}

fn raw_is_null(value: &ConstValue) -> bool {
    match value {
        ConstValue::Int { bits, .. } => *bits == 0,
        ConstValue::Ref { .. }
        | ConstValue::Slice { .. }
        | ConstValue::Str { .. }
        | ConstValue::Dyn { .. } => false,
        _ => false,
    }
}
