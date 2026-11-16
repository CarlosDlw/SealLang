//! Place address computation.

use inkwell::values::PointerValue;
use seal_mir::{Place, Projection};
use seal_ty::{TyId, TyKind};

use crate::error::CodegenError;
use crate::function::FunctionLowerer;

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    /// Computes the address of `place`, projecting through fields and shared
    /// references.
    pub(crate) fn place_ptr(&self, place: &Place) -> Result<PointerValue<'ctx>, CodegenError> {
        let mut pointer = self.alloca(place.local);
        let mut ty = self.local_type(place.local);
        for projection in &place.projections {
            match projection {
                Projection::Deref => {
                    let pointee = match self.tcx.kind(ty) {
                        TyKind::Ref { pointee, .. } => pointee,
                        other => {
                            return Err(CodegenError::internal(format!(
                                "cannot dereference `{other:?}` during code generation"
                            )));
                        }
                    };
                    let reference_type = self.mapper.basic(ty)?;
                    let reference = self.build(
                        self.builder
                            .build_load(reference_type, pointer, "dereference"),
                        "reference load",
                    )?;
                    pointer = match self.tcx.kind(pointee) {
                        TyKind::Slice { .. } => {
                            let fat = reference.into_struct_value();
                            self.build(
                                self.builder.build_extract_value(fat, 0, "slice data"),
                                "slice data",
                            )?
                            .into_pointer_value()
                        }
                        TyKind::Str => {
                            return Err(CodegenError::internal(
                                "a string reference cannot be dereferenced into a sized place",
                            ));
                        }
                        _ => reference.into_pointer_value(),
                    };
                    ty = pointee;
                }
                Projection::Field(field) => {
                    let TyKind::Adt(adt) = self.tcx.kind(ty) else {
                        return Err(CodegenError::internal(format!(
                            "cannot project a field from `{ty:?}` during code generation"
                        )));
                    };
                    let definition = self.tcx.adt(adt);
                    let index = definition
                        .fields
                        .iter()
                        .position(|candidate| candidate.def == *field)
                        .ok_or_else(|| {
                            CodegenError::internal(format!(
                                "field {field:?} does not belong to aggregate {adt:?}"
                            ))
                        })?;
                    let struct_type = self.mapper.adt(adt);
                    pointer = self.build(
                        self.builder
                            .build_struct_gep(struct_type, pointer, index as u32, "field"),
                        "field projection",
                    )?;
                    ty = definition.fields[index].ty;
                }
                Projection::Index { local, constant } => {
                    let element = match self.tcx.kind(ty) {
                        TyKind::Array { element, .. } | TyKind::Slice { element } => element,
                        _ => {
                            return Err(CodegenError::internal(format!(
                                "cannot index `{ty:?}` during code generation"
                            )));
                        }
                    };
                    let index = match constant {
                        Some(index) => self.mapper.usize_type()?.const_int(*index, false),
                        None => self
                            .load_operand(&seal_mir::Operand::Copy(Place::local(*local)))?
                            .into_int_value(),
                    };
                    pointer = self.index_ptr(pointer, index, element)?;
                    ty = element;
                }
            }
        }
        Ok(pointer)
    }

    /// Returns the address of `index` elements into an array.
    ///
    /// Inkwell exposes GEP only as an unsafe function, and the workspace forbids
    /// unsafe code outside the audited debug helper. Bounds checking happens in
    /// MIR before this point, so converting the pointer to an integer, adding a
    /// byte offset, and converting back computes the same address without an
    /// unsafe block. The array has a known layout and the index is in range, so
    /// the arithmetic cannot leave the allocation.
    pub(crate) fn index_ptr(
        &self,
        pointer: PointerValue<'ctx>,
        index: inkwell::values::IntValue<'ctx>,
        element: TyId,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let usize_type = self.mapper.usize_type()?;
        let element_size = self.mapper.layout(element)?.size();
        let base = self.build(
            self.builder
                .build_ptr_to_int(pointer, usize_type, "array base"),
            "array base address",
        )?;
        let offset = self.build(
            self.builder.build_int_mul(
                index,
                usize_type.const_int(element_size, false),
                "array offset",
            ),
            "array offset",
        )?;
        let address = self.build(
            self.builder.build_int_add(base, offset, "array address"),
            "array address",
        )?;
        self.build(
            self.builder
                .build_int_to_ptr(address, self.mapper.pointer(), "array element"),
            "array element address",
        )
    }

    /// Returns the type of the value stored at `place`.
    pub(crate) fn place_type(&self, place: &Place) -> Result<TyId, CodegenError> {
        let mut ty = self.local_type(place.local);
        for projection in &place.projections {
            match projection {
                Projection::Deref => {
                    let pointee = match self.tcx.kind(ty) {
                        TyKind::Ref { pointee, .. } => pointee,
                        other => {
                            return Err(CodegenError::internal(format!(
                                "cannot dereference `{other:?}` during code generation"
                            )));
                        }
                    };
                    if matches!(self.tcx.kind(pointee), TyKind::Str) {
                        return Err(CodegenError::internal(
                            "a string reference has no sized pointee",
                        ));
                    }
                    ty = pointee;
                }
                Projection::Field(field) => {
                    let TyKind::Adt(adt) = self.tcx.kind(ty) else {
                        return Err(CodegenError::internal(format!(
                            "cannot project a field from `{ty:?}` during code generation"
                        )));
                    };
                    let definition = self.tcx.adt(adt);
                    let index = definition
                        .fields
                        .iter()
                        .position(|candidate| candidate.def == *field)
                        .ok_or_else(|| {
                            CodegenError::internal(format!(
                                "field {field:?} does not belong to aggregate {adt:?}"
                            ))
                        })?;
                    ty = definition.fields[index].ty;
                }
                Projection::Index { .. } => {
                    ty = match self.tcx.kind(ty) {
                        TyKind::Array { element, .. } | TyKind::Slice { element } => element,
                        _ => {
                            return Err(CodegenError::internal(format!(
                                "cannot index `{ty:?}` during code generation"
                            )));
                        }
                    };
                }
            }
        }
        Ok(ty)
    }
}
