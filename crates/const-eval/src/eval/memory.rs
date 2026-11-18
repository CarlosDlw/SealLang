//! Place resolution, projection, and allocation access.

use super::*;

impl<'a> Interpreter<'a> {
    pub(super) fn resolve_place(
        &mut self,
        place: &Place,
        span: Span,
    ) -> Result<ResolvedPlace, EvalError> {
        let (alloc, ty) = match place.root {
            PlaceRoot::Local(local) => {
                let frame = self.frames.last().ok_or_else(|| {
                    self.error(EvalErrorKind::Internal("no frame for place".into()), span)
                })?;
                let alloc = *frame.locals.get(local.index() as usize).ok_or_else(|| {
                    self.error(
                        EvalErrorKind::Internal("local has no allocation".into()),
                        span,
                    )
                })?;
                (alloc, self.memory[alloc].ty)
            }
            PlaceRoot::Static(def) => {
                let alloc = self.ensure_static(def, span)?;
                (alloc, self.memory[alloc].ty)
            }
            PlaceRoot::Const(def) => {
                return self.internal(
                    format!(
                        "const `{}` reached evaluation as a place",
                        self.def_name(def)
                    ),
                    span,
                );
            }
        };
        let mut resolved = ResolvedPlace::at(alloc, ty);
        for projection in &place.projections {
            match projection {
                Projection::Field(field) => self.project_field(&mut resolved, *field, span)?,
                Projection::VariantField { variant, field } => {
                    self.project_variant_field(&mut resolved, *variant, *field, span)?
                }
                Projection::TupleField(index) => {
                    self.project_tuple_field(&mut resolved, *index, span)?
                }
                Projection::Index { local, constant } => {
                    let index = match constant {
                        Some(index) => *index,
                        None => self.read_local(*local, span)?.bits(),
                    };
                    self.project_index(&mut resolved, index, span)?;
                }
                Projection::Deref => self.project_deref(&mut resolved, span)?,
            }
        }
        Ok(resolved)
    }

    pub(super) fn project_field(
        &mut self,
        resolved: &mut ResolvedPlace,
        field: DefId,
        span: Span,
    ) -> Result<(), EvalError> {
        let TyKind::Adt { adt, args } = self.tcx.kind(resolved.ty) else {
            return self.internal("field projection on a non-aggregate", span);
        };
        let declaration = self.tcx.adt(adt);
        let index = declaration
            .fields
            .iter()
            .position(|candidate| candidate.def == field)
            .ok_or_else(|| {
                self.error(
                    EvalErrorKind::Internal("field is not part of its struct".into()),
                    span,
                )
            })?;
        let field_layout = self
            .layout
            .field_layout_with(adt, args, index)
            .map_err(|error| self.error(EvalErrorKind::Internal(error.to_string()), span))?;
        let field_ty = declaration.fields[index].ty;
        resolved.offset = resolved.offset.saturating_add(field_layout.offset);
        resolved.ty = field_ty;
        resolved.path.push(PathStep::Field(index));
        Ok(())
    }

    pub(super) fn project_variant_field(
        &mut self,
        resolved: &mut ResolvedPlace,
        variant: VariantIdx,
        field: DefId,
        span: Span,
    ) -> Result<(), EvalError> {
        let TyKind::Adt { adt, args } = self.tcx.kind(resolved.ty) else {
            return self.internal("variant field projection on a non-aggregate", span);
        };
        let index = place::variant_field_index(self.tcx, adt, variant, field).ok_or_else(|| {
            self.error(
                EvalErrorKind::Internal("field is not part of its variant".into()),
                span,
            )
        })?;
        let layout = self
            .layout
            .enum_layout_with(adt, args)
            .map_err(|error| self.error(EvalErrorKind::Internal(error.to_string()), span))?;
        let field_layout = layout
            .variants
            .get(variant.index() as usize)
            .and_then(|variant| variant.fields.get(index))
            .copied()
            .ok_or_else(|| {
                self.error(
                    EvalErrorKind::Internal("variant has no such field layout".into()),
                    span,
                )
            })?;
        let field_ty = self
            .tcx
            .variant_field_ty(adt, args, variant, field)
            .ok_or_else(|| {
                self.error(
                    EvalErrorKind::Internal("variant field has no type".into()),
                    span,
                )
            })?;
        resolved.offset = resolved.offset.saturating_add(field_layout.offset);
        resolved.ty = field_ty;
        resolved.path.push(PathStep::VariantField(index));
        Ok(())
    }

    pub(super) fn project_tuple_field(
        &mut self,
        resolved: &mut ResolvedPlace,
        index: u32,
        span: Span,
    ) -> Result<(), EvalError> {
        let layouts = self
            .layout
            .tuple_field_layouts(resolved.ty)
            .map_err(|error| self.error(EvalErrorKind::Internal(error.to_string()), span))?;
        let field_layout = layouts.get(index as usize).copied().ok_or_else(|| {
            self.error(
                EvalErrorKind::Internal("tuple has no such element".into()),
                span,
            )
        })?;
        let TyKind::Tuple(tuple) = self.tcx.kind(resolved.ty) else {
            return self.internal("tuple projection on a non-tuple", span);
        };
        let ty = self
            .tcx
            .tuple_element(tuple, index as usize)
            .ok_or_else(|| {
                self.error(
                    EvalErrorKind::Internal("tuple element has no type".into()),
                    span,
                )
            })?;
        resolved.offset = resolved.offset.saturating_add(field_layout.offset);
        resolved.ty = ty;
        resolved.path.push(PathStep::TupleField(index as usize));
        Ok(())
    }

    pub(super) fn project_index(
        &mut self,
        resolved: &mut ResolvedPlace,
        index: u64,
        span: Span,
    ) -> Result<(), EvalError> {
        if let Some(element) = resolved.slice {
            let length = resolved.slice_len.unwrap_or(0);
            if index >= length {
                return Err(self.error(EvalErrorKind::IndexOutOfBounds { index, length }, span));
            }
            let size = self
                .layout
                .layout_of(element)
                .map_err(|error| self.error(EvalErrorKind::Internal(error.to_string()), span))?
                .size();
            resolved.offset = resolved.offset.saturating_add(index.saturating_mul(size));
            resolved.ty = element;
            resolved.slice = None;
            resolved.slice_len = None;
            resolved.path = self.locate_path(resolved.alloc, resolved.offset, span)?;
            return Ok(());
        }
        let TyKind::Array { element, length } = self.tcx.kind(resolved.ty) else {
            return self.internal("index projection on a non-array", span);
        };
        if index >= length {
            return Err(self.error(EvalErrorKind::IndexOutOfBounds { index, length }, span));
        }
        let size = self
            .layout
            .layout_of(element)
            .map_err(|error| self.error(EvalErrorKind::Internal(error.to_string()), span))?
            .size();
        resolved.offset = resolved.offset.saturating_add(index.saturating_mul(size));
        resolved.ty = element;
        resolved.path.push(PathStep::Index(index as usize));
        Ok(())
    }

    pub(super) fn project_deref(
        &mut self,
        resolved: &mut ResolvedPlace,
        span: Span,
    ) -> Result<(), EvalError> {
        let value = self.read_place_raw(resolved, span)?;
        match value {
            ConstValue::Ref {
                target,
                offset,
                pointee,
                ..
            } => {
                resolved.alloc = target;
                resolved.offset = resolved.offset.saturating_add(offset);
                resolved.ty = pointee;
                resolved.slice = None;
                resolved.path = self.locate_path(target, resolved.offset, span)?;
            }
            ConstValue::Slice {
                target,
                offset,
                length,
                element,
            } => {
                resolved.alloc = target;
                resolved.offset = resolved.offset.saturating_add(offset);
                resolved.ty = self.tcx.slice_ty(element);
                resolved.slice = Some(element);
                resolved.slice_len = Some(length);
                resolved.path = self.locate_path(target, resolved.offset, span)?;
            }
            other => {
                return self.internal(
                    format!("dereference of a non-reference value {other:?}"),
                    span,
                );
            }
        }
        Ok(())
    }

    pub(super) fn locate_path(
        &self,
        alloc: AllocId,
        offset: u64,
        span: Span,
    ) -> Result<Vec<PathStep>, EvalError> {
        let allocation = &self.memory[alloc];
        let value = allocation.value.as_ref().ok_or_else(|| {
            self.error(
                EvalErrorKind::Internal("dereference of an uninitialized allocation".into()),
                span,
            )
        })?;
        place::locate(self.tcx, &self.layout, value, allocation.ty, offset).ok_or_else(|| {
            self.error(
                EvalErrorKind::Internal(format!(
                    "no value starts at byte offset {offset} in an allocation"
                )),
                span,
            )
        })
    }

    pub(super) fn ensure_static(&mut self, def: DefId, span: Span) -> Result<AllocId, EvalError> {
        if let Some(alloc) = self.static_allocs.get(&def).copied() {
            return Ok(alloc);
        }
        match self.states.get(&def) {
            Some(EvalState::Done) | Some(EvalState::Visiting) => {
                return self.internal("static has no allocation", span);
            }
            Some(EvalState::Failed) => {
                return self.internal("dependency failed to evaluate", span);
            }
            None => {}
        }
        self.record_dependency(def, DependencyKind::Address);
        self.eval_def(def)?;
        self.static_allocs.get(&def).copied().ok_or_else(|| {
            self.error(
                EvalErrorKind::Internal("static evaluation produced no allocation".into()),
                span,
            )
        })
    }

    pub(super) fn read_place_raw(
        &self,
        resolved: &ResolvedPlace,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        let allocation = &self.memory[resolved.alloc];
        let root = allocation.value.as_ref().ok_or_else(|| {
            self.error(
                EvalErrorKind::Internal("read of an uninitialized allocation".into()),
                span,
            )
        })?;
        place::read_at(root, &resolved.path)
            .cloned()
            .ok_or_else(|| {
                self.error(
                    EvalErrorKind::Internal("place path does not match the value".into()),
                    span,
                )
            })
    }

    pub(super) fn read_place(
        &self,
        resolved: &ResolvedPlace,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        self.read_place_raw(resolved, span)
    }

    pub(super) fn write_place(
        &mut self,
        resolved: &ResolvedPlace,
        value: ConstValue,
        span: Span,
    ) -> Result<(), EvalError> {
        if !self.memory[resolved.alloc].mutable {
            return self.internal("write through an immutable allocation", span);
        }
        let allocation = &mut self.memory[resolved.alloc];
        if resolved.path.is_empty() && allocation.value.is_none() {
            allocation.value = Some(value);
            return Ok(());
        }
        let written = match allocation.value.as_mut() {
            Some(root) => place::write_at(root, &resolved.path, value),
            None => false,
        };
        if written {
            Ok(())
        } else {
            self.internal("write to an invalid place", span)
        }
    }
}
