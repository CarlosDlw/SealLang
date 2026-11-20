//! Raw pointer method semantics during constant evaluation.
//!
//! A raw pointer keeps its allocation provenance, offset, metadata, and
//! mutability in a `Raw*` value. Pointees built from integers remain integers:
//! they have no allocation the evaluator can inspect, so any operation that
//! needs provenance fails with `E4032` instead of inventing an address.

use seal_methods::IntrinsicOp;
use seal_mir::{IntrinsicReceiver, Operand};
use seal_span::Span;
use seal_symbol::Symbol;
use seal_ty::TyId;

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
            IntrinsicOp::PtrWithLen | IntrinsicOp::PtrAsStr => {
                Err(self.error(EvalErrorKind::RawPointerAccess, span))
            }
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
                    _ => Err(self.error(EvalErrorKind::AddressNotComputable, span)),
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
            ConstValue::RawSlice { length, .. } | ConstValue::Slice { length, .. } => *length,
            ConstValue::RawStr { value, .. } | ConstValue::Str { value, .. } => {
                self.interner.resolve(*value).len() as u64
            }
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
        let (_, pointee) = self.tcx.raw_parts(result_ty).ok_or_else(|| {
            self.error(
                EvalErrorKind::Internal("pointer data result is not a raw pointer".into()),
                span,
            )
        })?;
        match value {
            ConstValue::RawSlice {
                target,
                offset,
                element,
                mutable,
                ..
            } => Ok(ConstValue::RawPtr {
                target,
                offset,
                pointee: element,
                mutable,
            }),
            ConstValue::Slice {
                target,
                offset,
                element,
                ..
            } => Ok(ConstValue::RawPtr {
                target,
                offset,
                pointee: element,
                mutable: false,
            }),
            ConstValue::RawPtr { .. } | ConstValue::Int { .. } => Ok(value),
            ConstValue::RawDyn {
                data,
                data_offset,
                mutable,
                ..
            } => Ok(ConstValue::RawPtr {
                target: data,
                offset: data_offset,
                pointee,
                mutable,
            }),
            ConstValue::Ref {
                target,
                offset,
                pointee,
                kind,
            } => Ok(ConstValue::RawPtr {
                target,
                offset,
                pointee,
                mutable: kind == seal_ty::RefKind::Exclusive,
            }),
            ConstValue::RawStr { value, mutable } => Ok(ConstValue::RawStrData {
                value,
                offset: 0,
                mutable,
            }),
            ConstValue::RawStrData { .. } => Ok(value),
            ConstValue::Str { .. } | ConstValue::Dyn { .. } => {
                Err(self.error(EvalErrorKind::RawPointerAccess, span))
            }
            _ => Err(self.error(EvalErrorKind::RawPointerAccess, span)),
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
            ConstValue::RawPtr {
                target,
                offset,
                pointee,
                mutable,
            } => {
                let offset = self.raw_next(target, offset, delta, in_bounds, span)?;
                Ok(ConstValue::RawPtr {
                    target,
                    offset,
                    pointee,
                    mutable,
                })
            }
            ConstValue::RawSlice {
                target,
                offset,
                length,
                element,
                mutable,
            } => {
                let offset = self.raw_next(target, offset, delta, in_bounds, span)?;
                Ok(ConstValue::RawSlice {
                    target,
                    offset,
                    length,
                    element,
                    mutable,
                })
            }
            ConstValue::RawDyn {
                data,
                data_offset,
                concrete,
                trait_ref,
                mutable,
            } => {
                let data_offset = self.raw_next(data, data_offset, delta, in_bounds, span)?;
                Ok(ConstValue::RawDyn {
                    data,
                    data_offset,
                    concrete,
                    trait_ref,
                    mutable,
                })
            }
            ConstValue::RawStrData {
                value,
                offset,
                mutable,
            } => {
                let next = i128::from(offset).wrapping_add(delta);
                let length = self.interner.resolve(value).len() as i128;
                if in_bounds && (next < 0 || next > length) {
                    return Err(self.error(EvalErrorKind::RawPointerAccess, span));
                }
                Ok(ConstValue::RawStrData {
                    value,
                    offset: next as u64,
                    mutable,
                })
            }
            // A string pointer carries metadata but no allocation of its own.
            ConstValue::RawStr { .. } => Err(self.error(EvalErrorKind::RawPointerAccess, span)),
            _ => Err(self.error(EvalErrorKind::RawPointerAccess, span)),
        }
    }

    /// Advances an allocation-backed pointer, rejecting addresses outside the
    /// allocation for the checked operations.
    fn raw_next(
        &self,
        target: AllocId,
        offset: u64,
        delta: i128,
        in_bounds: bool,
        span: Span,
    ) -> Result<u64, EvalError> {
        let next = i128::from(offset).wrapping_add(delta);
        if in_bounds && (next < 0 || next > i128::from(self.memory[target].size)) {
            return Err(self.error(EvalErrorKind::RawPointerAccess, span));
        }
        Ok(next as u64)
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
        let left = self
            .pointer_location(&value)
            .ok_or_else(|| self.error(EvalErrorKind::RawPointerAccess, span))?;
        let right = self
            .pointer_location(&other)
            .ok_or_else(|| self.error(EvalErrorKind::RawPointerAccess, span))?;
        let (left, right, pointee) = match (left, right) {
            (
                PointerLocation::Allocation {
                    target: left_target,
                    offset: left,
                    pointee,
                },
                PointerLocation::Allocation {
                    target: right_target,
                    offset: right,
                    ..
                },
            ) if left_target == right_target => (left, right, pointee),
            (
                PointerLocation::String {
                    value: left,
                    offset: left_offset,
                },
                PointerLocation::String {
                    value: right,
                    offset: right_offset,
                },
            ) if left == right => (
                left_offset,
                right_offset,
                self.tcx.uint_ty(seal_ty::UintTy::U8),
            ),
            _ => return Err(self.error(EvalErrorKind::RawPointerAccess, span)),
        };
        let size = self
            .layout
            .layout_of(pointee)
            .map_err(|error| self.error(EvalErrorKind::Internal(error.to_string()), span))?
            .size();
        let difference = i128::from(left) - i128::from(right);
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

    fn pointer_location(&self, value: &ConstValue) -> Option<PointerLocation> {
        match value {
            ConstValue::RawPtr {
                target,
                offset,
                pointee,
                ..
            } => Some(PointerLocation::Allocation {
                target: *target,
                offset: *offset,
                pointee: *pointee,
            }),
            ConstValue::RawSlice {
                target,
                offset,
                element,
                ..
            } => Some(PointerLocation::Allocation {
                target: *target,
                offset: *offset,
                pointee: *element,
            }),
            ConstValue::RawDyn {
                data,
                data_offset,
                concrete,
                ..
            } => Some(PointerLocation::Allocation {
                target: *data,
                offset: *data_offset,
                pointee: *concrete,
            }),
            ConstValue::RawStrData { value, offset, .. } => Some(PointerLocation::String {
                value: *value,
                offset: *offset,
            }),
            _ => None,
        }
    }

    fn raw_read(&mut self, value: ConstValue, span: Span) -> Result<ConstValue, EvalError> {
        if let ConstValue::RawStrData {
            value: text,
            offset,
            ..
        } = value
        {
            let bytes = self.interner.resolve(text);
            let index = usize::try_from(offset)
                .map_err(|_| self.error(EvalErrorKind::RawPointerAccess, span))?;
            let byte = *bytes
                .as_bytes()
                .get(index)
                .ok_or_else(|| self.error(EvalErrorKind::RawPointerAccess, span))?;
            let ty = self.tcx.uint_ty(seal_ty::UintTy::U8);
            return Ok(ConstValue::Int {
                bits: u128::from(byte),
                ty,
            });
        }
        let ConstValue::RawPtr {
            target,
            offset,
            pointee,
            ..
        } = value
        else {
            return Err(self.error(EvalErrorKind::RawPointerAccess, span));
        };
        let allocation = &self.memory[target];
        let layout = self
            .layout
            .layout_of(pointee)
            .map_err(|error| self.error(EvalErrorKind::Internal(error.to_string()), span))?;
        if offset > allocation.size || layout.size() > allocation.size - offset {
            return Err(self.error(EvalErrorKind::RawPointerAccess, span));
        }
        if layout.align() > 1 && offset % layout.align() != 0 {
            return Err(self.error(EvalErrorKind::RawPointerAccess, span));
        }
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
    /// the two cases. A missing path is a user-visible invalid access, not an
    /// internal invariant failure.
    fn view_path(
        &self,
        target: AllocId,
        offset: u64,
        pointee: TyId,
        span: Span,
    ) -> Result<Vec<PathStep>, EvalError> {
        let allocation = &self.memory[target];
        let root = allocation
            .value
            .as_ref()
            .ok_or_else(|| self.error(EvalErrorKind::RawPointerAccess, span))?;
        let path = if offset == 0 && allocation.ty != pointee {
            place::locate_member(self.tcx, &self.layout, root, allocation.ty, offset)
        } else {
            place::locate(self.tcx, &self.layout, root, allocation.ty, offset)
        };
        path.ok_or_else(|| self.error(EvalErrorKind::RawPointerAccess, span))
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

enum PointerLocation {
    Allocation {
        target: AllocId,
        offset: u64,
        pointee: TyId,
    },
    String {
        value: Symbol,
        offset: u64,
    },
}

fn raw_is_null(value: &ConstValue) -> bool {
    match value {
        ConstValue::Int { bits, .. } => *bits == 0,
        ConstValue::RawPtr { .. }
        | ConstValue::RawSlice { .. }
        | ConstValue::RawStr { .. }
        | ConstValue::RawStrData { .. }
        | ConstValue::RawDyn { .. }
        | ConstValue::Ref { .. }
        | ConstValue::Slice { .. }
        | ConstValue::Str { .. }
        | ConstValue::Dyn { .. } => false,
        _ => false,
    }
}
