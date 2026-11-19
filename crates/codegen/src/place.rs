//! Place address computation.

use inkwell::values::PointerValue;
use seal_mir::{Place, Projection};
use seal_ty::{TyId, TyKind};

use crate::error::CodegenError;
use crate::function::FunctionLowerer;

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    /// Computes the address of `place`, projecting through fields and shared
    /// references.
    ///
    /// A local root is a stack slot and a static root is the address of its
    /// global. A const root never reaches code generation: MIR materializes it
    /// into a temporary, so a const root here is an invariant violation.
    pub(crate) fn place_ptr(&self, place: &Place) -> Result<PointerValue<'ctx>, CodegenError> {
        let (mut pointer, mut ty) = match place.root {
            seal_mir::PlaceRoot::Local(local) => (self.alloca(local), self.local_type(local)),
            seal_mir::PlaceRoot::Static(def) => {
                let ty = self.globals.static_type(def).ok_or_else(|| {
                    CodegenError::internal(format!(
                        "static {def:?} has no declared global during code generation"
                    ))
                })?;
                (self.globals.static_pointer(def)?, ty)
            }
            seal_mir::PlaceRoot::Const(def) => {
                return Err(CodegenError::internal(format!(
                    "place rooted at const {def:?} was not materialized into a temporary"
                )));
            }
        };
        for projection in &place.projections {
            match projection {
                Projection::Deref | Projection::RawDeref => {
                    let pointee = match self.tcx.kind(ty) {
                        TyKind::Ref { pointee, .. } | TyKind::RawPtr { pointee, .. } => pointee,
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
                        TyKind::Dyn { .. } => {
                            return Err(CodegenError::internal(
                                "a trait-object reference has no sized place; reborrow it instead",
                            ));
                        }
                        _ => reference.into_pointer_value(),
                    };
                    ty = pointee;
                }
                Projection::Field(field) => {
                    let TyKind::Adt { adt, args } = self.tcx.kind(ty) else {
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
                    let struct_type = self.mapper.adt(adt, args)?;
                    pointer = self.build(
                        self.builder
                            .build_struct_gep(struct_type, pointer, index as u32, "field"),
                        "field projection",
                    )?;
                    ty = self.tcx.substitute(definition.fields[index].ty, args);
                }
                Projection::TupleField(index) => {
                    let TyKind::Tuple(tuple) = self.tcx.kind(ty) else {
                        return Err(CodegenError::internal(format!(
                            "cannot project a tuple element from `{ty:?}` during code generation"
                        )));
                    };
                    let element =
                        self.tcx
                            .tuple_element(tuple, *index as usize)
                            .ok_or_else(|| {
                                CodegenError::internal(format!(
                                    "tuple element {index} is outside {ty:?}"
                                ))
                            })?;
                    let tuple_type = self.mapper.basic(ty)?.into_struct_type();
                    pointer = self.build(
                        self.builder
                            .build_struct_gep(tuple_type, pointer, *index, "tuple element"),
                        "tuple element projection",
                    )?;
                    ty = element;
                }
                Projection::VariantField { variant, field } => {
                    let TyKind::Adt { adt, args } = self.tcx.kind(ty) else {
                        return Err(CodegenError::internal(format!(
                            "cannot project an enum field from `{ty:?}` during code generation"
                        )));
                    };
                    let definition = self.tcx.adt(adt);
                    let variant_def = definition.variant(*variant).ok_or_else(|| {
                        CodegenError::internal(format!(
                            "variant {variant:?} does not belong to enum {adt:?}"
                        ))
                    })?;
                    let index = variant_def
                        .fields
                        .iter()
                        .position(|candidate| candidate.def == *field)
                        .ok_or_else(|| {
                            CodegenError::internal(format!(
                                "field {field:?} does not belong to variant {variant:?}"
                            ))
                        })?;
                    let layout = self.mapper.enum_layout(adt, args)?;
                    let offset = layout
                        .variants
                        .get(variant.index() as usize)
                        .and_then(|variant| variant.fields.get(index))
                        .map(|field| field.offset)
                        .ok_or_else(|| {
                            CodegenError::internal(format!(
                                "enum {adt:?} has no layout for variant {variant:?} field {index}"
                            ))
                        })?;
                    pointer = self.offset_ptr(pointer, offset)?;
                    ty = self.tcx.substitute(variant_def.fields[index].ty, args);
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

    /// Returns the address `offset` bytes past `pointer`.
    ///
    /// The same integer arithmetic used by array indexing avoids an unsafe GEP
    /// call. Enum field offsets come from the language layout, so the result is
    /// inside the enum allocation.
    fn offset_ptr(
        &self,
        pointer: PointerValue<'ctx>,
        offset: u64,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let usize_type = self.mapper.usize_type()?;
        let base = self.build(
            self.builder
                .build_ptr_to_int(pointer, usize_type, "field base"),
            "field base address",
        )?;
        let address = self.build(
            self.builder
                .build_int_add(base, usize_type.const_int(offset, false), "field address"),
            "field address",
        )?;
        self.build(
            self.builder
                .build_int_to_ptr(address, self.mapper.pointer(), "field pointer"),
            "field pointer",
        )
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
        let mut ty = match place.root {
            seal_mir::PlaceRoot::Local(local) => self.local_type(local),
            // A static's storage is the authoritative source of its type, so a
            // missing global is an invariant violation. A const root still
            // resolves through HIR even though MIR should have materialized it.
            seal_mir::PlaceRoot::Static(def) => self.globals.static_type(def).ok_or_else(|| {
                CodegenError::internal(format!(
                    "static {def:?} has no declared global during code generation"
                ))
            })?,
            seal_mir::PlaceRoot::Const(def) => self
                .hir
                .body_of(def)
                .and_then(|body| body.ret)
                .unwrap_or_else(|| self.tcx.error_ty()),
        };
        for projection in &place.projections {
            match projection {
                Projection::Deref | Projection::RawDeref => {
                    let pointee = match self.tcx.kind(ty) {
                        TyKind::Ref { pointee, .. } | TyKind::RawPtr { pointee, .. } => pointee,
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
                    let TyKind::Adt { adt, args } = self.tcx.kind(ty) else {
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
                    ty = self.tcx.substitute(definition.fields[index].ty, args);
                }
                Projection::TupleField(index) => {
                    let TyKind::Tuple(tuple) = self.tcx.kind(ty) else {
                        return Err(CodegenError::internal(format!(
                            "cannot project a tuple element from `{ty:?}` during code generation"
                        )));
                    };
                    ty = self
                        .tcx
                        .tuple_element(tuple, *index as usize)
                        .ok_or_else(|| {
                            CodegenError::internal(format!(
                                "tuple element {index} is outside {ty:?}"
                            ))
                        })?;
                }
                Projection::VariantField { variant, field } => {
                    let TyKind::Adt { adt, args } = self.tcx.kind(ty) else {
                        return Err(CodegenError::internal(format!(
                            "cannot project an enum field from `{ty:?}` during code generation"
                        )));
                    };
                    let definition = self.tcx.adt(adt);
                    let variant_def = definition.variant(*variant).ok_or_else(|| {
                        CodegenError::internal(format!(
                            "variant {variant:?} does not belong to enum {adt:?}"
                        ))
                    })?;
                    let field = variant_def.field(*field).ok_or_else(|| {
                        CodegenError::internal(format!(
                            "field {field:?} does not belong to variant {variant:?}"
                        ))
                    })?;
                    ty = self.tcx.substitute(field.ty, args);
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
