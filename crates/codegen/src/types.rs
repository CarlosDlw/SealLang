//! Mapping Seal types to LLVM types and layouts.
//!
//! Aggregates are named LLVM structs so a reference to a struct can be created
//! before the struct's fields are known. Every aggregate is reserved as an
//! opaque struct first and filled in a second pass, which is what makes `*T`
//! fields and mutually referencing structs work without unsafe pointer casts.
//!
//! The LLVM struct layout matches the language layout for every supported
//! scalar: fields keep declaration order, each one starts at its natural
//! alignment, and the struct is padded to its maximum alignment. Scalars use
//! their size as alignment, references use the target pointer width, and `bool`
//! is a one-byte `i1`.

use std::cell::RefCell;

use inkwell::AddressSpace;
use inkwell::context::Context;
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, FunctionType, StructType};
use seal_hash::FxHashMap;
use seal_index::Idx;
use seal_layout::{Layout, LayoutCtxt, LayoutError};
use seal_symbol::Interner;
use seal_ty::{AdtId, AdtKind, FloatTy, IntTy, TupleId, TyCtxt, TyId, TyKind, UintTy};

use crate::error::CodegenError;

/// Maps interned Seal types to LLVM types.
pub(crate) struct TypeMapper<'ctx> {
    context: &'ctx Context,
    tcx: &'ctx TyCtxt,
    layouts: LayoutCtxt<'ctx>,
    pointer_width: u8,
    empty: StructType<'ctx>,
    fat_pointer: StructType<'ctx>,
    adts: Vec<StructType<'ctx>>,
    tuples: RefCell<FxHashMap<TupleId, StructType<'ctx>>>,
}

impl<'ctx> TypeMapper<'ctx> {
    /// Creates the mapper and builds every aggregate type.
    pub(crate) fn new(
        context: &'ctx Context,
        tcx: &'ctx TyCtxt,
        interner: &Interner,
        pointer_width: u8,
    ) -> Result<Self, CodegenError> {
        let empty = context.struct_type(&[], false);
        let byte_ptr = context.ptr_type(AddressSpace::default());
        let usize_ty = match pointer_width {
            32 => context.i32_type(),
            64 => context.i64_type(),
            other => {
                return Err(CodegenError::internal(format!(
                    "unsupported pointer width `{other}`"
                )));
            }
        };
        let fat_pointer = context.struct_type(&[byte_ptr.into(), usize_ty.into()], false);
        let layouts = LayoutCtxt::new(tcx, pointer_width);

        let mut adts = Vec::with_capacity(tcx.adt_len());
        for index in 0..tcx.adt_len() {
            let adt = AdtId::from_usize(index);
            let definition = tcx.adt(adt);
            let name = format!("seal.adt.{index}.{}", interner.resolve(definition.name));
            adts.push(context.opaque_struct_type(&name));
        }

        let mapper = Self {
            context,
            tcx,
            layouts,
            pointer_width,
            empty,
            fat_pointer,
            adts,
            tuples: RefCell::new(FxHashMap::default()),
        };
        mapper.fill_aggregates()?;
        Ok(mapper)
    }

    fn fill_aggregates(&self) -> Result<(), CodegenError> {
        for index in 0..self.adts.len() {
            let adt = AdtId::from_usize(index);
            let definition = self.tcx.adt(adt);
            let field_types = match definition.kind {
                AdtKind::Struct => {
                    let mut field_types = Vec::with_capacity(definition.fields.len());
                    for field in &definition.fields {
                        field_types.push(self.basic(field.ty)?);
                    }
                    field_types
                }
                AdtKind::Enum => self.enum_fields(adt)?,
            };
            self.adts[index].set_body(&field_types, false);
        }
        Ok(())
    }

    /// Builds the LLVM body of an enum: the tag, an alignment anchor, and the
    /// payload bytes.
    ///
    /// The zero-length array of a payload-aligned integer contributes the
    /// payload's alignment without occupying storage, so the payload array lands
    /// exactly at the language layout's payload offset. Variant fields are later
    /// stored into the payload through byte offsets, which keeps one LLVM type
    /// able to hold every variant.
    fn enum_fields(&self, adt: AdtId) -> Result<Vec<BasicTypeEnum<'ctx>>, CodegenError> {
        let layout = self.enum_layout(adt)?;
        let tag_bits = u16::try_from(layout.tag.size() * 8).map_err(|_| {
            CodegenError::internal(format!("enum {adt:?} has a tag too wide for LLVM"))
        })?;
        let anchor_bits = u16::try_from(layout.payload_align * 8).map_err(|_| {
            CodegenError::internal(format!(
                "enum {adt:?} has a payload alignment too wide for LLVM"
            ))
        })?;
        let payload_size = u32::try_from(layout.payload_size).map_err(|_| {
            CodegenError::internal(format!("enum {adt:?} has a payload too large for LLVM"))
        })?;
        let tag = self.integer(Some(tag_bits))?;
        let anchor = self.integer(Some(anchor_bits))?.array_type(0);
        let payload = self.context.i8_type().array_type(payload_size);
        Ok(vec![tag.into(), anchor.into(), payload.into()])
    }

    /// Returns the LLVM type of `ty`.
    pub(crate) fn basic(&self, ty: TyId) -> Result<BasicTypeEnum<'ctx>, CodegenError> {
        let basic = match self.tcx.kind(ty) {
            TyKind::Bool => BasicTypeEnum::IntType(self.context.bool_type()),
            TyKind::Char => BasicTypeEnum::IntType(self.context.i32_type()),
            TyKind::Int(int) => BasicTypeEnum::IntType(self.integer(int.bits())?),
            TyKind::Uint(uint) => BasicTypeEnum::IntType(self.integer(uint.bits())?),
            TyKind::Float(float) => BasicTypeEnum::FloatType(match float {
                FloatTy::F32 => self.context.f32_type(),
                FloatTy::F64 => self.context.f64_type(),
            }),
            TyKind::Ref { pointee, .. } => match self.tcx.kind(pointee) {
                // `*str` and `*[T]` are fat pointers: data plus length.
                TyKind::Str | TyKind::Slice { .. } => BasicTypeEnum::StructType(self.fat_pointer),
                _ => BasicTypeEnum::PointerType(self.pointer()),
            },
            TyKind::Array { element, length } => {
                let length = u32::try_from(length).map_err(|_| {
                    CodegenError::internal(format!(
                        "array length {length} does not fit an LLVM array type"
                    ))
                })?;
                BasicTypeEnum::ArrayType(self.basic(element)?.array_type(length))
            }
            TyKind::Adt(adt) => BasicTypeEnum::StructType(self.adt(adt)),
            TyKind::Tuple(tuple) => {
                let cached = self.tuples.borrow().get(&tuple).copied();
                let tuple_type = match cached {
                    Some(tuple_type) => tuple_type,
                    None => {
                        let mut fields = Vec::new();
                        for element in self.tcx.tuple_elements(tuple) {
                            fields.push(self.basic(element)?);
                        }
                        let created = self.context.struct_type(&fields, false);
                        self.tuples.borrow_mut().insert(tuple, created);
                        created
                    }
                };
                BasicTypeEnum::StructType(tuple_type)
            }
            TyKind::Unit | TyKind::Never | TyKind::Error => BasicTypeEnum::StructType(self.empty),
            TyKind::Str | TyKind::Slice { .. } => {
                return Err(CodegenError::internal(
                    "an unsized type has no LLVM value type; only its reference is representable",
                ));
            }
            TyKind::Infer(_) => {
                return Err(CodegenError::internal(
                    "an unresolved inference type reached code generation",
                ));
            }
        };
        Ok(basic)
    }

    /// Returns the function type with the given parameter and return types.
    pub(crate) fn function_type(
        &self,
        parameters: &[TyId],
        returns: TyId,
    ) -> Result<FunctionType<'ctx>, CodegenError> {
        let mut llvm_parameters = Vec::with_capacity(parameters.len());
        for parameter in parameters {
            llvm_parameters.push(BasicMetadataTypeEnum::from(self.basic(*parameter)?));
        }
        Ok(self.basic(returns)?.fn_type(&llvm_parameters, false))
    }

    /// Returns the named struct type for `adt`.
    pub(crate) fn adt(&self, adt: AdtId) -> StructType<'ctx> {
        self.adts[adt.index() as usize]
    }

    /// Returns the layout of `ty`.
    pub(crate) fn layout(&self, ty: TyId) -> Result<Layout, CodegenError> {
        self.layouts.layout_of(ty).map_err(layout_error)
    }

    /// Returns the tag and payload layout of the enum `adt`.
    pub(crate) fn enum_layout(&self, adt: AdtId) -> Result<seal_layout::EnumLayout, CodegenError> {
        self.layouts.enum_layout(adt).map_err(layout_error)
    }

    /// Returns an integer type of `bits` bits.
    pub(crate) fn integer_type(
        &self,
        bits: u16,
    ) -> Result<inkwell::types::IntType<'ctx>, CodegenError> {
        self.integer(Some(bits))
    }

    /// Returns the layout of every field of `adt`, in declaration order.
    pub(crate) fn field_layouts(
        &self,
        adt: AdtId,
    ) -> Result<Vec<seal_layout::FieldLayout>, CodegenError> {
        self.layouts.field_layouts(adt).map_err(layout_error)
    }

    /// Returns the layout of every tuple element, in declaration order.
    pub(crate) fn tuple_field_layouts(
        &self,
        ty: TyId,
    ) -> Result<Vec<seal_layout::FieldLayout>, CodegenError> {
        self.layouts.tuple_field_layouts(ty).map_err(layout_error)
    }

    /// Returns the zero-sized struct used for `()`, `!`, and the error type.
    pub(crate) fn empty_type(&self) -> StructType<'ctx> {
        self.empty
    }

    /// Returns the pointer type of the target.
    pub(crate) fn pointer(&self) -> inkwell::types::PointerType<'ctx> {
        self.context.ptr_type(AddressSpace::default())
    }

    /// Returns the pointer-sized integer type.
    pub(crate) fn usize_type(&self) -> Result<inkwell::types::IntType<'ctx>, CodegenError> {
        self.integer(Some(u16::from(self.pointer_width)))
    }

    /// Returns `true` for types represented by a zero-sized LLVM struct.
    pub(crate) fn is_unit_like(&self, ty: TyId) -> bool {
        matches!(
            self.tcx.kind(ty),
            TyKind::Unit | TyKind::Never | TyKind::Error
        )
    }

    /// Returns the pointer width in bits.
    pub(crate) fn pointer_width(&self) -> u8 {
        self.pointer_width
    }

    fn integer(&self, bits: Option<u16>) -> Result<inkwell::types::IntType<'ctx>, CodegenError> {
        let bits = bits.unwrap_or(u16::from(self.pointer_width));
        Ok(match bits {
            8 => self.context.i8_type(),
            16 => self.context.i16_type(),
            32 => self.context.i32_type(),
            64 => self.context.i64_type(),
            128 => self.context.i128_type(),
            other => {
                return Err(CodegenError::internal(format!(
                    "unsupported integer width `{other}`"
                )));
            }
        })
    }
}

fn layout_error(error: LayoutError) -> CodegenError {
    CodegenError::internal(format!("type has no usable layout: {error}"))
}

/// Returns the width of an integer type in bits.
pub(crate) fn int_width(int: IntTy, pointer_width: u8) -> u16 {
    int.bits().unwrap_or(u16::from(pointer_width))
}

/// Returns the width of an unsigned integer type in bits.
pub(crate) fn uint_width(uint: UintTy, pointer_width: u8) -> u16 {
    uint.bits().unwrap_or(u16::from(pointer_width))
}

#[cfg(test)]
mod tests {
    use inkwell::context::Context;
    use seal_symbol::Interner;
    use seal_ty::TyCtxt;

    use super::TypeMapper;

    #[test]
    fn a_slice_reference_is_a_fat_pointer() {
        let context = Context::create();
        let tcx = TyCtxt::new();
        let interner = Interner::new();
        let int = tcx.int_ty(seal_ty::IntTy::I32);
        let slice = tcx.ref_ty(tcx.slice_ty(int));

        let mapper =
            TypeMapper::new(&context, &tcx, &interner, 64).expect("pointer width is supported");
        let fat = mapper
            .basic(slice)
            .expect("a slice reference is sized")
            .into_struct_type();
        assert_eq!(fat.get_field_types().len(), 2);
    }

    #[test]
    fn a_string_reference_length_matches_the_pointer_width() {
        let context = Context::create();
        let tcx = TyCtxt::new();
        let interner = Interner::new();
        let string = tcx.ref_ty(tcx.str_ty());

        for (pointer_width, expected_bits) in [(32, 32), (64, 64)] {
            let mapper = TypeMapper::new(&context, &tcx, &interner, pointer_width)
                .expect("pointer width is supported");
            let fat = mapper
                .basic(string)
                .expect("a string reference is sized")
                .into_struct_type();
            let fields = fat.get_field_types();
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[1].into_int_type().get_bit_width(), expected_bits);
        }
    }
}
