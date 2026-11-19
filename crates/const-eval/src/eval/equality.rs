//! Structural and content equality.
//!
//! Const evaluation must agree with code generation, so equality follows the
//! same rules here: aggregates compare field by field or variant by variant,
//! and `*str` and `*[T]` compare their contents. Thin references and trait
//! objects never reach this point because type checking rejects them.

use seal_mir::BinOp;
use seal_span::Span;
use seal_ty::TyId;

use super::*;

/// Names a value's representation for internal invariant messages.
fn shape_of(value: &ConstValue) -> &'static str {
    match value {
        ConstValue::Int { .. } => "an integer",
        ConstValue::Float { .. } => "a float",
        ConstValue::Bool(_) => "a bool",
        ConstValue::Char(_) => "a char",
        ConstValue::Unit => "the unit value",
        ConstValue::Tuple(_) => "a tuple",
        ConstValue::Array(_) => "an array",
        ConstValue::Struct { .. } => "a struct",
        ConstValue::Variant { .. } => "an enum variant",
        ConstValue::Str { .. } => "a string reference",
        ConstValue::Ref { .. } => "a reference",
        ConstValue::Slice { .. } => "a slice reference",
        ConstValue::Dyn { .. } => "a trait object",
        ConstValue::RawPtr { .. } => "a raw pointer",
        ConstValue::RawSlice { .. } => "a raw slice pointer",
        ConstValue::RawStr { .. } => "a raw string pointer",
        ConstValue::RawStrData { .. } => "a raw string data pointer",
        ConstValue::RawDyn { .. } => "a raw trait object",
    }
}

impl<'a> Interpreter<'a> {
    /// Compares two values of the same type, returning `lhs == rhs`.
    pub(super) fn values_equal(
        &self,
        lhs: &ConstValue,
        rhs: &ConstValue,
        span: Span,
    ) -> Result<bool, EvalError> {
        match (lhs, rhs) {
            // Type checking gives both operands the same type, so integer
            // equality is bit equality of their normalized representations.
            (ConstValue::Int { bits: lhs, .. }, ConstValue::Int { bits: rhs, .. }) => {
                Ok(lhs == rhs)
            }
            (
                ConstValue::Float {
                    bits: lhs,
                    ty: lhs_ty,
                },
                ConstValue::Float { bits: rhs, .. },
            ) => {
                let width = self.float_width(*lhs_ty, span)?;
                Ok(ops::float_compare(
                    BinOp::Eq,
                    ops::float_value(*lhs, width),
                    ops::float_value(*rhs, width),
                ))
            }
            (ConstValue::Bool(lhs), ConstValue::Bool(rhs)) => Ok(lhs == rhs),
            (ConstValue::Char(lhs), ConstValue::Char(rhs)) => Ok(lhs == rhs),
            (ConstValue::Unit, ConstValue::Unit) => Ok(true),
            (ConstValue::Tuple(lhs), ConstValue::Tuple(rhs)) => {
                self.sequences_equal(lhs, rhs, span)
            }
            (ConstValue::Array(lhs), ConstValue::Array(rhs)) => {
                self.sequences_equal(lhs, rhs, span)
            }
            (ConstValue::Struct { fields: lhs, .. }, ConstValue::Struct { fields: rhs, .. }) => {
                self.fields_equal(lhs, rhs, span)
            }
            (
                ConstValue::Variant {
                    variant: lhs_variant,
                    fields: lhs_fields,
                    ..
                },
                ConstValue::Variant {
                    variant: rhs_variant,
                    fields: rhs_fields,
                    ..
                },
            ) => {
                if lhs_variant != rhs_variant {
                    return Ok(false);
                }
                self.fields_equal(lhs_fields, rhs_fields, span)
            }
            (ConstValue::Str { value: lhs, .. }, ConstValue::Str { value: rhs, .. }) => {
                Ok(self.interner.resolve(*lhs) == self.interner.resolve(*rhs))
            }
            (
                ConstValue::Slice {
                    target: lhs_target,
                    offset: lhs_offset,
                    length: lhs_length,
                    element,
                },
                ConstValue::Slice {
                    target: rhs_target,
                    offset: rhs_offset,
                    length: rhs_length,
                    ..
                },
            ) => self.slice_contents_equal(
                *element,
                (*lhs_target, *lhs_offset, *lhs_length),
                (*rhs_target, *rhs_offset, *rhs_length),
                span,
            ),
            (
                ConstValue::RawPtr {
                    target: lhs_target,
                    offset: lhs_offset,
                    ..
                },
                ConstValue::RawPtr {
                    target: rhs_target,
                    offset: rhs_offset,
                    ..
                },
            ) => Ok(lhs_target == rhs_target && lhs_offset == rhs_offset),
            (
                ConstValue::RawSlice {
                    target: lhs_target,
                    offset: lhs_offset,
                    ..
                },
                ConstValue::RawSlice {
                    target: rhs_target,
                    offset: rhs_offset,
                    ..
                },
            ) => Ok(lhs_target == rhs_target && lhs_offset == rhs_offset),
            (ConstValue::RawStr { value: lhs, .. }, ConstValue::RawStr { value: rhs, .. }) => {
                Ok(lhs == rhs)
            }
            (
                ConstValue::RawStrData {
                    value: lhs,
                    offset: lhs_offset,
                    ..
                },
                ConstValue::RawStrData {
                    value: rhs,
                    offset: rhs_offset,
                    ..
                },
            ) => Ok(lhs == rhs && lhs_offset == rhs_offset),
            (
                ConstValue::RawDyn {
                    data: lhs_data,
                    data_offset: lhs_offset,
                    ..
                },
                ConstValue::RawDyn {
                    data: rhs_data,
                    data_offset: rhs_offset,
                    ..
                },
            ) => Ok(lhs_data == rhs_data && lhs_offset == rhs_offset),
            // An integer-built pointer has no allocation provenance. It can
            // only compare equal to another integer-built pointer, which is
            // handled by the integer case above.
            (
                ConstValue::RawPtr { .. }
                | ConstValue::RawSlice { .. }
                | ConstValue::RawStr { .. }
                | ConstValue::RawStrData { .. }
                | ConstValue::RawDyn { .. },
                ConstValue::Int { .. },
            )
            | (
                ConstValue::Int { .. },
                ConstValue::RawPtr { .. }
                | ConstValue::RawSlice { .. }
                | ConstValue::RawStr { .. }
                | ConstValue::RawStrData { .. }
                | ConstValue::RawDyn { .. },
            ) => Ok(false),
            (ConstValue::Ref { .. }, ConstValue::Ref { .. }) => {
                self.internal("a thin reference has no equality in const evaluation", span)
            }
            (ConstValue::Dyn { .. }, ConstValue::Dyn { .. }) => {
                self.internal("a trait object has no equality in const evaluation", span)
            }
            _ => {
                let lhs = shape_of(lhs);
                let rhs = shape_of(rhs);
                self.internal(
                    format!("equality operands have mismatched shapes: {lhs} and {rhs}"),
                    span,
                )
            }
        }
    }

    /// Compares two element sequences, such as tuples and arrays.
    fn sequences_equal(
        &self,
        lhs: &[ConstValue],
        rhs: &[ConstValue],
        span: Span,
    ) -> Result<bool, EvalError> {
        if lhs.len() != rhs.len() {
            return self.internal("equality operands have different lengths", span);
        }
        for (lhs, rhs) in lhs.iter().zip(rhs) {
            if !self.values_equal(lhs, rhs, span)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Compares the fields of two values of the same aggregate.
    fn fields_equal(
        &self,
        lhs: &[(seal_def::DefId, ConstValue)],
        rhs: &[(seal_def::DefId, ConstValue)],
        span: Span,
    ) -> Result<bool, EvalError> {
        if lhs.len() != rhs.len() {
            return self.internal("equality operands have different field counts", span);
        }
        for ((_, lhs), (_, rhs)) in lhs.iter().zip(rhs) {
            if !self.values_equal(lhs, rhs, span)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Compares two slice views by length and element contents.
    fn slice_contents_equal(
        &self,
        element: TyId,
        lhs: (AllocId, u64, u64),
        rhs: (AllocId, u64, u64),
        span: Span,
    ) -> Result<bool, EvalError> {
        let (lhs_target, lhs_offset, lhs_length) = lhs;
        let (rhs_target, rhs_offset, rhs_length) = rhs;
        if lhs_length != rhs_length {
            return Ok(false);
        }
        // Elements of a zero-sized type carry no state, so the lengths already
        // decide the comparison and no byte offset can address an element.
        let size = self
            .layout
            .layout_of(element)
            .map_err(|error| self.error(EvalErrorKind::Internal(error.to_string()), span))?
            .size();
        if size == 0 {
            return Ok(true);
        }
        for index in 0..lhs_length {
            let offset = index.saturating_mul(size);
            let lhs =
                self.value_at(lhs_target, lhs_offset.saturating_add(offset), element, span)?;
            let rhs =
                self.value_at(rhs_target, rhs_offset.saturating_add(offset), element, span)?;
            if !self.values_equal(&lhs, &rhs, span)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Reads the value of type `ty` that starts at byte `offset` of an
    /// allocation.
    ///
    /// An allocation that holds exactly `ty` has the value at offset zero;
    /// otherwise the offset addresses a member of the stored aggregate.
    fn value_at(
        &self,
        allocation: AllocId,
        offset: u64,
        ty: TyId,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        let stored = &self.memory[allocation];
        let root = stored.value.as_ref().ok_or_else(|| {
            self.error(
                EvalErrorKind::Internal("read of an uninitialized allocation".into()),
                span,
            )
        })?;
        let path = if stored.ty == ty {
            Vec::new()
        } else {
            place::locate_member(self.tcx, &self.layout, root, stored.ty, offset).ok_or_else(
                || {
                    self.error(
                        EvalErrorKind::Internal(format!(
                            "no value starts at byte offset {offset} in an allocation"
                        )),
                        span,
                    )
                },
            )?
        };
        place::read_at(root, &path).cloned().ok_or_else(|| {
            self.error(
                EvalErrorKind::Internal("place path does not match the value".into()),
                span,
            )
        })
    }
}
