//! Rvalue evaluation and scalar operator semantics.

use super::*;

impl<'a> Interpreter<'a> {
    pub(super) fn eval_rvalue(
        &mut self,
        rvalue: &Rvalue,
        span: Span,
        destination: TyId,
    ) -> Result<ConstValue, EvalError> {
        match rvalue {
            Rvalue::Use(operand) => self.read_operand(operand, span),
            Rvalue::Cast {
                kind,
                operand,
                from,
                to,
            } => {
                let value = self.read_operand(operand, span)?;
                let seal_ty::CastKind::Numeric(kind) = *kind else {
                    return self.raw_cast(*kind, value, *to, span);
                };
                let (from_bits, from_signed) = if self.is_integer(*from) {
                    self.integer_info(*from, span)?
                } else {
                    (self.float_width(*from, span)?, false)
                };
                let (to_bits, to_signed) = if self.is_integer(*to) {
                    self.integer_info(*to, span)?
                } else {
                    (self.float_width(*to, span)?, false)
                };
                ops::cast(
                    kind,
                    &value,
                    from_bits,
                    from_signed,
                    to_bits,
                    to_signed,
                    *to,
                )
                .map_err(|kind| self.error(kind, span))
            }
            Rvalue::Ref { place, kind } => {
                let resolved = self.resolve_place(place, span)?;
                let (alloc, offset) = self.borrow_target(place, &resolved, span)?;
                if let PlaceRoot::Static(def) = place.root {
                    self.record_dependency(def, DependencyKind::Address);
                }
                Ok(ConstValue::Ref {
                    target: alloc,
                    offset,
                    pointee: resolved.ty,
                    kind: *kind,
                })
            }
            Rvalue::SliceRef {
                base,
                start,
                end,
                kind: _,
                ty,
            } => {
                let resolved = self.resolve_place(base, span)?;
                let length = self.base_length(&resolved, span)?;
                let start = match start {
                    Some(operand) => {
                        let value = self.read_operand(operand, span)?;
                        self.as_u128(&value, span)? as u64
                    }
                    None => 0,
                };
                let end = match end {
                    Some(operand) => {
                        let value = self.read_operand(operand, span)?;
                        self.as_u128(&value, span)? as u64
                    }
                    None => length,
                };
                if start > end || end > length {
                    return Err(self.error(EvalErrorKind::SliceRange { start, end, length }, span));
                }
                let element = match self.tcx.kind(*ty) {
                    TyKind::Ref { pointee, .. } => match self.tcx.kind(pointee) {
                        TyKind::Slice { element } => element,
                        _ => return self.internal("slice reference has a non-slice pointee", span),
                    },
                    _ => return self.internal("slice reference is not a reference type", span),
                };
                let (alloc, offset) = self.borrow_target(base, &resolved, span)?;
                let size = self
                    .layout
                    .layout_of(element)
                    .map_err(|error| self.error(EvalErrorKind::Internal(error.to_string()), span))?
                    .size();
                Ok(ConstValue::Slice {
                    target: alloc,
                    offset: offset.saturating_add(start.saturating_mul(size)),
                    length: end - start,
                    element,
                })
            }
            Rvalue::Discriminant(place) => {
                let resolved = self.resolve_place(place, span)?;
                let value = self.read_place(&resolved, span)?;
                match value {
                    ConstValue::Variant { variant, .. } => Ok(ConstValue::Int {
                        bits: u128::from(variant.index()),
                        ty: destination,
                    }),
                    _ => self.internal("discriminant of a non-enum value", span),
                }
            }
            Rvalue::Len { slice } => {
                let value = self.read_operand(slice, span)?;
                let length = match value {
                    ConstValue::Slice { length, .. } => length,
                    ConstValue::Str { value, .. } => self.interner.resolve(value).len() as u64,
                    _ => return self.internal("length of a non-slice value", span),
                };
                Ok(ConstValue::Int {
                    bits: u128::from(length),
                    ty: destination,
                })
            }
            Rvalue::Binary { op, lhs, rhs, .. } => {
                let lhs = self.read_operand(lhs, span)?;
                let rhs = self.read_operand(rhs, span)?;
                self.binary(*op, &lhs, &rhs, destination, span)
            }
            Rvalue::Unary { op, operand } => {
                let value = self.read_operand(operand, span)?;
                self.unary(*op, &value, destination, span)
            }
            Rvalue::Aggregate { kind, fields } => {
                let mut values = Vec::with_capacity(fields.len());
                for (field, operand) in fields {
                    values.push((*field, self.read_operand(operand, span)?));
                }
                match kind {
                    AggregateKind::Struct { adt, args } => Ok(ConstValue::Struct {
                        adt: *adt,
                        args: *args,
                        fields: values,
                    }),
                    AggregateKind::Variant { adt, variant, args } => Ok(ConstValue::Variant {
                        adt: *adt,
                        variant: *variant,
                        args: *args,
                        fields: values,
                    }),
                }
            }
            Rvalue::Tuple { elements, ty } => {
                let mut values = Vec::with_capacity(elements.len());
                for element in elements {
                    values.push(self.read_operand(element, span)?);
                }
                let _ = ty;
                if values.is_empty() {
                    Ok(ConstValue::Unit)
                } else {
                    Ok(ConstValue::Tuple(values))
                }
            }
            Rvalue::Array { elements, ty } => {
                let mut values = Vec::with_capacity(elements.len());
                for element in elements {
                    values.push(self.read_operand(element, span)?);
                }
                let _ = ty;
                Ok(ConstValue::Array(values))
            }
            Rvalue::Repeat { value, count, ty } => {
                let value = self.read_operand(value, span)?;
                let _ = ty;
                Ok(ConstValue::Array(vec![value; *count as usize]))
            }
            Rvalue::MakeTraitObject {
                source,
                source_ty,
                target_ty,
            } => {
                let _ = source_ty;
                let value = self.read_operand(source, span)?;
                self.make_dyn_value(value, *target_ty, span)
            }
            Rvalue::TraitObjectReborrow { source, .. } => self.read_operand(source, span),
            Rvalue::Call { target, args, .. } => self.eval_call(*target, args, span),
            Rvalue::Intrinsic {
                method,
                receiver,
                args,
                ..
            } => self.eval_intrinsic(*method, receiver, args, destination, span),
            Rvalue::DynamicCall { .. } => Err(self.error(
                EvalErrorKind::DynamicDispatch {
                    const_fn: self.caller_is_const_fn(),
                },
                span,
            )),
        }
    }

    pub(super) fn borrow_target(
        &mut self,
        place: &Place,
        resolved: &ResolvedPlace,
        span: Span,
    ) -> Result<(AllocId, u64), EvalError> {
        let _ = place;
        if let PlaceRoot::Local(local) = place.root {
            let frame = self.frames.last().ok_or_else(|| {
                self.error(EvalErrorKind::Internal("no frame for borrow".into()), span)
            })?;
            let is_temp = self
                .mir_body_local_kind(local)
                .is_some_and(|kind| kind == seal_mir::LocalKind::Temp);
            if is_temp {
                let index = self.promoted.entry(frame.owner).or_insert(0);
                let promoted_index = *index;
                *index += 1;
                let source = AllocSource::Promoted {
                    owner: frame.owner,
                    index: promoted_index,
                };
                let allocation = &self.memory[resolved.alloc];
                let value = allocation.value.clone().ok_or_else(|| {
                    self.error(
                        EvalErrorKind::Internal("promoted temporary is uninitialized".into()),
                        span,
                    )
                })?;
                let ty = allocation.ty;
                let size = allocation.size;
                let alloc = self.allocate(ty, Some(value), false, source, size, span)?;
                return Ok((alloc, resolved.offset));
            }
        }
        Ok((resolved.alloc, resolved.offset))
    }

    pub(super) fn mir_body_local_kind(
        &self,
        local: seal_mir::LocalId,
    ) -> Option<seal_mir::LocalKind> {
        let body = self.mir.body_of(self.current_owner()?)?;
        Some(body.local(local).kind)
    }

    pub(super) fn current_owner(&self) -> Option<DefId> {
        self.frames.last().map(|frame| frame.owner)
    }

    pub(super) fn base_length(
        &self,
        resolved: &ResolvedPlace,
        span: Span,
    ) -> Result<u64, EvalError> {
        if let Some(length) = resolved.slice_len {
            return Ok(length);
        }
        match self.tcx.kind(resolved.ty) {
            TyKind::Array { length, .. } => Ok(length),
            _ => self.internal("slice source is not an array or slice", span),
        }
    }

    pub(super) fn binary(
        &self,
        op: seal_mir::BinOp,
        lhs: &ConstValue,
        rhs: &ConstValue,
        destination: TyId,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        match (lhs, rhs) {
            (
                ConstValue::Int {
                    bits: lhs_bits,
                    ty: lhs_ty,
                },
                ConstValue::Int { bits: rhs_bits, .. },
            ) => {
                let (bits, signed) = self.integer_info(*lhs_ty, span)?;
                if matches!(
                    op,
                    seal_mir::BinOp::Eq
                        | seal_mir::BinOp::Ne
                        | seal_mir::BinOp::Lt
                        | seal_mir::BinOp::Gt
                        | seal_mir::BinOp::Le
                        | seal_mir::BinOp::Ge
                ) {
                    return Ok(ConstValue::Bool(ops::int_compare(
                        op, *lhs_bits, *rhs_bits, bits, signed,
                    )));
                }
                let _ = destination;
                ops::int_binary(op, *lhs_bits, *rhs_bits, bits, signed, *lhs_ty)
                    .map_err(|kind| self.error(kind, span))
            }
            (
                ConstValue::Float {
                    bits: lhs_bits,
                    ty: lhs_ty,
                },
                ConstValue::Float { bits: rhs_bits, .. },
            ) => {
                let width = self.float_width(*lhs_ty, span)?;
                ops::float_binary(op, *lhs_bits, *rhs_bits, width, *lhs_ty)
                    .map_err(|kind| self.error(kind, span))
            }
            (ConstValue::Bool(lhs), ConstValue::Bool(rhs)) => match op {
                seal_mir::BinOp::Eq => Ok(ConstValue::Bool(lhs == rhs)),
                seal_mir::BinOp::Ne => Ok(ConstValue::Bool(lhs != rhs)),
                _ => self.internal("operator is not defined for booleans", span),
            },
            (ConstValue::Char(lhs), ConstValue::Char(rhs)) => {
                let result = match op {
                    seal_mir::BinOp::Eq => lhs == rhs,
                    seal_mir::BinOp::Ne => lhs != rhs,
                    seal_mir::BinOp::Lt => lhs < rhs,
                    seal_mir::BinOp::Gt => lhs > rhs,
                    seal_mir::BinOp::Le => lhs <= rhs,
                    seal_mir::BinOp::Ge => lhs >= rhs,
                    _ => return self.internal("operator is not defined for chars", span),
                };
                Ok(ConstValue::Bool(result))
            }
            _ => {
                if !matches!(op, seal_mir::BinOp::Eq | seal_mir::BinOp::Ne) {
                    return self.internal("binary operands have mismatched scalar kinds", span);
                }
                let equal = self.values_equal(lhs, rhs, span)?;
                Ok(ConstValue::Bool(if matches!(op, seal_mir::BinOp::Ne) {
                    !equal
                } else {
                    equal
                }))
            }
        }
    }

    pub(super) fn unary(
        &self,
        op: UnOp,
        value: &ConstValue,
        destination: TyId,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        match value {
            ConstValue::Int { bits, ty } => {
                let (width, _) = self.integer_info(*ty, span)?;
                ops::int_unary(op, *bits, width, destination).map_err(|kind| self.error(kind, span))
            }
            ConstValue::Float { bits, ty } => {
                let width = self.float_width(*ty, span)?;
                let value = ops::float_value(*bits, width);
                match op {
                    UnOp::Neg => Ok(ConstValue::Float {
                        bits: ops::float_bits(-value, width),
                        ty: destination,
                    }),
                    _ => self.internal("operator is not defined for floats", span),
                }
            }
            ConstValue::Bool(value) if op == UnOp::Not => Ok(ConstValue::Bool(!value)),
            _ => self.internal("unary operand has an unexpected scalar kind", span),
        }
    }

    pub(super) fn is_integer(&self, ty: TyId) -> bool {
        matches!(self.tcx.kind(ty), TyKind::Int(_) | TyKind::Uint(_))
    }

    /// Evaluates the pointer conversions that are not numeric.
    fn raw_cast(
        &mut self,
        kind: seal_ty::CastKind,
        value: ConstValue,
        to: TyId,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        match kind {
            seal_ty::CastKind::Numeric(_) | seal_ty::CastKind::RawToRef => {
                self.internal("a non-pointer cast reached the raw-pointer evaluator", span)
            }
            seal_ty::CastKind::RefToRaw { .. } => {
                let unsizes_to_object = matches!(
                    self.tcx.kind(to),
                    TyKind::RawPtr { pointee, .. }
                        if matches!(self.tcx.kind(pointee), TyKind::Dyn { .. })
                );
                if unsizes_to_object {
                    match value {
                        ConstValue::Ref { .. } => self.make_dyn_value(value, to, span),
                        other => Ok(other),
                    }
                } else {
                    Ok(value)
                }
            }
            seal_ty::CastKind::RawToRaw => Ok(value),
            seal_ty::CastKind::IntToRaw => match value {
                ConstValue::Int { bits, .. } => Ok(ConstValue::Int { bits, ty: to }),
                other => self.internal(
                    format!("an integer-to-pointer cast received {other:?}"),
                    span,
                ),
            },
            seal_ty::CastKind::RawToInt => match value {
                ConstValue::Int { bits, .. } => Ok(ConstValue::Int { bits, ty: to }),
                _ => Err(self.error(EvalErrorKind::AddressNotComputable, span)),
            },
        }
    }

    /// Builds the fat pointer of a trait object from a thin source reference.
    fn make_dyn_value(
        &mut self,
        value: ConstValue,
        target_ty: TyId,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        let (data, data_offset, concrete) = match value {
            ConstValue::Ref {
                target,
                offset,
                pointee,
                ..
            } => (target, offset, pointee),
            _ => return self.internal("trait object data is not a reference", span),
        };
        let object = match self.tcx.kind(target_ty) {
            TyKind::Ref { pointee, .. } | TyKind::RawPtr { pointee, .. } => pointee,
            _ => target_ty,
        };
        let principal = *self.tcx.dyn_trait_refs(object).first().ok_or_else(|| {
            self.error(
                EvalErrorKind::Internal("trait object has no principal trait".into()),
                span,
            )
        })?;
        // The dyn type records its own unsized self type. A vtable key is always
        // built from the concrete pointee, exactly as a runtime coercion does,
        // so the constant and the coercion name the same table.
        let trait_ref = TraitRef::new(principal.trait_def, concrete, principal.args);
        let key = TraitObjectKey {
            concrete,
            trait_ref,
        };
        if !self.vtables.contains(&key) {
            self.vtables.push(key);
        }
        Ok(ConstValue::Dyn {
            data,
            data_offset,
            concrete,
            trait_ref,
        })
    }
}
