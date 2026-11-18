//! Array and slice method semantics.

use super::*;
use crate::place::{self, PathStep};

/// One array or slice viewed as a sequence of elements.
struct Sequence {
    alloc: AllocId,
    base: u64,
    length: u64,
    element: TyId,
    /// Steps from the allocation root for an array receiver; `None` for a slice.
    path: Option<Vec<PathStep>>,
}

impl<'a> Interpreter<'a> {
    pub(super) fn sequence_intrinsic(
        &mut self,
        intrinsic: IntrinsicOp,
        receiver: &IntrinsicReceiver,
        args: &[Operand],
        result_ty: TyId,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        let sequence = self.sequence(receiver, span)?;
        match intrinsic {
            IntrinsicOp::Len => Ok(ConstValue::Int {
                bits: u128::from(sequence.length),
                ty: result_ty,
            }),
            IntrinsicOp::IsEmpty => Ok(ConstValue::Bool(sequence.length == 0)),
            IntrinsicOp::Swap => {
                let first = self.int_argument(&args[0], span)? as usize;
                let second = self.int_argument(&args[1], span)? as usize;
                self.check_bounds(&sequence, first, span)?;
                self.check_bounds(&sequence, second, span)?;
                let first_value = self.read_element(&sequence, first, span)?;
                let second_value = self.read_element(&sequence, second, span)?;
                self.write_element(&sequence, first, second_value, span)?;
                self.write_element(&sequence, second, first_value, span)?;
                Ok(ConstValue::Unit)
            }
            IntrinsicOp::Reverse => {
                let length = sequence.length as usize;
                for index in 0..length / 2 {
                    let mirror = length - 1 - index;
                    let first = self.read_element(&sequence, index, span)?;
                    let second = self.read_element(&sequence, mirror, span)?;
                    self.write_element(&sequence, index, second, span)?;
                    self.write_element(&sequence, mirror, first, span)?;
                }
                Ok(ConstValue::Unit)
            }
            IntrinsicOp::Fill => {
                let value = self.read_operand(&args[0], span)?;
                for index in 0..sequence.length as usize {
                    self.write_element(&sequence, index, value.clone(), span)?;
                }
                Ok(ConstValue::Unit)
            }
            _ => Err(self.error(
                EvalErrorKind::Internal("scalar intrinsic in the sequence path".into()),
                span,
            )),
        }
    }

    fn sequence(
        &mut self,
        receiver: &IntrinsicReceiver,
        span: Span,
    ) -> Result<Sequence, EvalError> {
        match receiver {
            IntrinsicReceiver::Place(place) => {
                let mut resolved = self.resolve_place(place, span)?;
                if let TyKind::Ref { pointee, .. } = self.tcx.kind(resolved.ty)
                    && matches!(self.tcx.kind(pointee), TyKind::Slice { .. })
                {
                    self.project_deref(&mut resolved, span)?;
                }
                if let TyKind::Array { element, length } = self.tcx.kind(resolved.ty) {
                    return Ok(Sequence {
                        alloc: resolved.alloc,
                        base: resolved.offset,
                        length,
                        element,
                        path: Some(resolved.path),
                    });
                }
                if let Some(element) = resolved.slice {
                    return Ok(Sequence {
                        alloc: resolved.alloc,
                        base: resolved.offset,
                        length: resolved.slice_len.unwrap_or(0),
                        element,
                        path: None,
                    });
                }
                Err(self.error(
                    EvalErrorKind::Internal("sequence receiver is not an array or slice".into()),
                    span,
                ))
            }
            IntrinsicReceiver::Value(operand) => match self.read_operand(operand, span)? {
                ConstValue::Slice {
                    target,
                    offset,
                    length,
                    element,
                    ..
                } => Ok(Sequence {
                    alloc: target,
                    base: offset,
                    length,
                    element,
                    path: None,
                }),
                _ => Err(self.error(
                    EvalErrorKind::Internal("slice receiver is not a slice".into()),
                    span,
                )),
            },
        }
    }

    fn check_bounds(&self, sequence: &Sequence, index: usize, span: Span) -> Result<(), EvalError> {
        if index as u64 >= sequence.length {
            return Err(self.error(
                EvalErrorKind::IndexOutOfBounds {
                    index: index as u64,
                    length: sequence.length,
                },
                span,
            ));
        }
        Ok(())
    }

    fn element_path(
        &self,
        sequence: &Sequence,
        index: usize,
        span: Span,
    ) -> Result<Vec<PathStep>, EvalError> {
        if let Some(mut path) = sequence.path.clone() {
            path.push(PathStep::Index(index));
            return Ok(path);
        }
        let size = self
            .layout
            .layout_of(sequence.element)
            .map_err(|error| self.error(EvalErrorKind::Internal(error.to_string()), span))?
            .size();
        let offset = sequence
            .base
            .saturating_add((index as u64).saturating_mul(size));
        self.locate_path(sequence.alloc, offset, span)
    }

    fn read_element(
        &self,
        sequence: &Sequence,
        index: usize,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        let path = self.element_path(sequence, index, span)?;
        let allocation = &self.memory[sequence.alloc];
        let root = allocation.value.as_ref().ok_or_else(|| {
            self.error(
                EvalErrorKind::Internal("sequence allocation is uninitialized".into()),
                span,
            )
        })?;
        place::read_at(root, &path).cloned().ok_or_else(|| {
            self.error(
                EvalErrorKind::Internal("sequence element path is invalid".into()),
                span,
            )
        })
    }

    fn write_element(
        &mut self,
        sequence: &Sequence,
        index: usize,
        value: ConstValue,
        span: Span,
    ) -> Result<(), EvalError> {
        let path = self.element_path(sequence, index, span)?;
        if !self.memory[sequence.alloc].mutable {
            return Err(self.error(
                EvalErrorKind::Internal("write through an immutable sequence".into()),
                span,
            ));
        }
        let allocation = &mut self.memory[sequence.alloc];
        let written = match allocation.value.as_mut() {
            Some(root) => place::write_at(root, &path, value),
            None => false,
        };
        if written {
            Ok(())
        } else {
            Err(self.error(
                EvalErrorKind::Internal("sequence element path is invalid".into()),
                span,
            ))
        }
    }
}
