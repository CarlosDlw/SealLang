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
                        TyKind::Ref(pointee) => pointee,
                        other => {
                            return Err(CodegenError::internal(format!(
                                "cannot dereference `{other:?}` during code generation"
                            )));
                        }
                    };
                    if matches!(self.tcx.kind(pointee), TyKind::Str) {
                        return Err(CodegenError::internal(
                            "a string reference cannot be dereferenced into a sized place",
                        ));
                    }
                    let reference_type = self.mapper.basic(ty)?;
                    let reference = self.build(
                        self.builder
                            .build_load(reference_type, pointer, "dereference"),
                        "reference load",
                    )?;
                    pointer = reference.into_pointer_value();
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
            }
        }
        Ok(pointer)
    }

    /// Returns the type of the value stored at `place`.
    pub(crate) fn place_type(&self, place: &Place) -> Result<TyId, CodegenError> {
        let mut ty = self.local_type(place.local);
        for projection in &place.projections {
            match projection {
                Projection::Deref => {
                    let pointee = match self.tcx.kind(ty) {
                        TyKind::Ref(pointee) => pointee,
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
            }
        }
        Ok(ty)
    }
}
