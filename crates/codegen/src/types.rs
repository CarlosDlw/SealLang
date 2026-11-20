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

use std::cell::{Cell, RefCell};

use inkwell::AddressSpace;
use inkwell::context::Context;
use inkwell::types::{BasicMetadataTypeEnum, BasicType, BasicTypeEnum, FunctionType, StructType};
use seal_hash::FxHashMap;
use seal_index::Idx;
use seal_layout::{Layout, LayoutCtxt, LayoutError};
use seal_symbol::Interner;
use seal_ty::{AdtId, AdtKind, FloatTy, IntTy, SubstsId, TupleId, TyCtxt, TyId, TyKind, UintTy};

use crate::error::CodegenError;

/// Maps interned Seal types to LLVM types.
///
/// Generic aggregates are mapped per instantiation: `Pair<i32, bool>` and
/// `Pair<u8, i32>` receive distinct LLVM structs with their own field types and
/// layouts. Each struct is reserved as opaque before its body is filled, so a
/// reference to it can be created first, and a recursive reference finds the
/// reservation instead of recursing forever.
pub(crate) struct TypeMapper<'ctx> {
    context: &'ctx Context,
    tcx: &'ctx TyCtxt,
    interner: &'ctx Interner,
    layouts: LayoutCtxt<'ctx>,
    pointer_width: u8,
    empty: StructType<'ctx>,
    fat_pointer: StructType<'ctx>,
    dyn_pointer: StructType<'ctx>,
    adts: RefCell<FxHashMap<(AdtId, SubstsId), StructType<'ctx>>>,
    vtables: RefCell<FxHashMap<usize, StructType<'ctx>>>,
    next_instantiation: Cell<u32>,
    tuples: RefCell<FxHashMap<TupleId, StructType<'ctx>>>,
}

impl<'ctx> TypeMapper<'ctx> {
    /// Creates the mapper and builds every aggregate type.
    pub(crate) fn new(
        context: &'ctx Context,
        tcx: &'ctx TyCtxt,
        interner: &'ctx Interner,
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
        // A trait-object pointer is a data pointer followed by its vtable
        // pointer. Both members are opaque pointers, so the pair has exactly
        // two pointer-sized words on every supported target.
        let dyn_pointer = context.struct_type(&[byte_ptr.into(), byte_ptr.into()], false);
        let layouts = LayoutCtxt::new(tcx, pointer_width);

        let mapper = Self {
            context,
            tcx,
            interner,
            layouts,
            pointer_width,
            empty,
            fat_pointer,
            dyn_pointer,
            adts: RefCell::new(FxHashMap::default()),
            vtables: RefCell::new(FxHashMap::default()),
            next_instantiation: Cell::new(0),
            tuples: RefCell::new(FxHashMap::default()),
        };
        mapper.fill_non_generic_aggregates()?;
        Ok(mapper)
    }

    /// Builds every aggregate that declares no type parameters.
    ///
    /// A generic aggregate has no single LLVM type: it is built lazily for each
    /// `(definition, type arguments)` pair the program actually uses. Concrete
    /// aggregates keep their eager construction so a malformed definition is
    /// reported before any function body is lowered.
    fn fill_non_generic_aggregates(&self) -> Result<(), CodegenError> {
        for index in 0..self.tcx.adt_len() {
            let adt = AdtId::from_usize(index);
            if self.tcx.adt_generic_count(adt) != 0 {
                continue;
            }
            self.adt(adt, self.tcx.empty_substs())?;
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
    fn enum_fields(
        &self,
        adt: AdtId,
        args: SubstsId,
    ) -> Result<Vec<BasicTypeEnum<'ctx>>, CodegenError> {
        let layout = self.enum_layout(adt, args)?;
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
            TyKind::Ref { pointee, .. } | TyKind::RawPtr { pointee, .. } => {
                self.pointer_type(pointee)
            }
            TyKind::Array { element, length } => {
                let length = u32::try_from(length).map_err(|_| {
                    CodegenError::internal(format!(
                        "array length {length} does not fit an LLVM array type"
                    ))
                })?;
                BasicTypeEnum::ArrayType(self.basic(element)?.array_type(length))
            }
            TyKind::Adt { adt, args } => BasicTypeEnum::StructType(self.adt(adt, args)?),
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
            TyKind::Str | TyKind::Slice { .. } | TyKind::Dyn { .. } => {
                return Err(CodegenError::internal(
                    "an unsized type has no LLVM value type; only its reference is representable",
                ));
            }
            TyKind::Infer(_) => {
                return Err(CodegenError::internal(
                    "an unresolved inference type reached code generation",
                ));
            }
            TyKind::Param(_) => {
                return Err(CodegenError::internal(
                    "an uninstantiated type parameter reached code generation",
                ));
            }
        };
        Ok(basic)
    }

    /// Returns the LLVM type of a pointer to `pointee`.
    ///
    /// References and raw pointers share one representation: an opaque
    /// pointer, a `{ data, length }` pair for `str` and slices, and a
    /// `{ data, vtable }` pair for trait objects.
    fn pointer_type(&self, pointee: TyId) -> BasicTypeEnum<'ctx> {
        match self.tcx.kind(pointee) {
            TyKind::Str | TyKind::Slice { .. } => BasicTypeEnum::StructType(self.fat_pointer),
            TyKind::Dyn { .. } => BasicTypeEnum::StructType(self.dyn_pointer),
            _ => BasicTypeEnum::PointerType(self.pointer()),
        }
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

    /// Returns the C function type with the given parameter and return types.
    ///
    /// A Seal `()` return is C `void`, not the zero-sized struct a Seal
    /// function returns, so an external declaration maps the two differently.
    pub(crate) fn c_function_type(
        &self,
        parameters: &[TyId],
        returns: TyId,
    ) -> Result<FunctionType<'ctx>, CodegenError> {
        let mut llvm_parameters = Vec::with_capacity(parameters.len());
        for parameter in parameters {
            llvm_parameters.push(BasicMetadataTypeEnum::from(self.basic(*parameter)?));
        }
        if self.is_unit_like(returns) {
            Ok(self.context.void_type().fn_type(&llvm_parameters, false))
        } else {
            Ok(self.basic(returns)?.fn_type(&llvm_parameters, false))
        }
    }

    /// Returns the LLVM struct of `adt` instantiated with `args`.
    ///
    /// The struct is created on first use, reserved as opaque, then filled from
    /// the instantiated field types. A non-generic aggregate uses the empty
    /// argument list and keeps the historical `seal.adt.<index>.<name>` type
    /// name; each generic instantiation receives a unique suffix.
    pub(crate) fn adt(&self, adt: AdtId, args: SubstsId) -> Result<StructType<'ctx>, CodegenError> {
        let key = (adt, args);
        if let Some(found) = self.adts.borrow().get(&key).copied() {
            return Ok(found);
        }
        let definition = self.tcx.adt(adt);
        let name = if self.tcx.substs_args(args).is_empty() {
            format!(
                "seal.adt.{}.{}",
                adt.index(),
                self.interner.resolve(definition.name)
            )
        } else {
            let serial = self.next_instantiation.get();
            self.next_instantiation.set(serial + 1);
            format!(
                "seal.adt.{}.{}.{serial}",
                adt.index(),
                self.interner.resolve(definition.name)
            )
        };
        let created = self.context.opaque_struct_type(&name);
        self.adts.borrow_mut().insert(key, created);

        let field_types = match definition.kind {
            AdtKind::Struct => {
                let mut field_types = Vec::with_capacity(definition.fields.len());
                for field in &definition.fields {
                    let field_ty = self.tcx.substitute(field.ty, args);
                    field_types.push(self.basic(field_ty)?);
                }
                field_types
            }
            AdtKind::Enum => self.enum_fields(adt, args)?,
        };
        created.set_body(&field_types, false);
        Ok(created)
    }

    /// Returns the layout of `ty`.
    pub(crate) fn layout(&self, ty: TyId) -> Result<Layout, CodegenError> {
        self.layouts.layout_of(ty).map_err(layout_error)
    }

    /// Returns the tag and payload layout of the enum `adt` instantiated with
    /// `args`.
    pub(crate) fn enum_layout(
        &self,
        adt: AdtId,
        args: SubstsId,
    ) -> Result<seal_layout::EnumLayout, CodegenError> {
        self.layouts
            .enum_layout_with(adt, args)
            .map_err(layout_error)
    }

    /// Returns an integer type of `bits` bits.
    pub(crate) fn integer_type(
        &self,
        bits: u16,
    ) -> Result<inkwell::types::IntType<'ctx>, CodegenError> {
        self.integer(Some(bits))
    }

    /// Returns the layout of every field of `adt` instantiated with `args`, in
    /// declaration order.
    pub(crate) fn field_layouts(
        &self,
        adt: AdtId,
        args: SubstsId,
    ) -> Result<Vec<seal_layout::FieldLayout>, CodegenError> {
        self.layouts
            .field_layouts_with(adt, args)
            .map_err(layout_error)
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

    /// Returns the vtable struct with `slots` pointer-sized entries.
    ///
    /// Slots 0..3 are the drop pointer, size, and alignment header; the rest
    /// are method pointers. The struct shape follows the language's vtable
    /// layout so every slot can be addressed with a checked struct GEP instead
    /// of an unsafe byte offset. Types are cached by slot count because every
    /// vtable with the same number of entries has the same LLVM type.
    pub(crate) fn vtable_type(&self, slots: usize) -> Result<StructType<'ctx>, CodegenError> {
        if slots < seal_layout::VTABLE_HEADER_SLOTS as usize {
            return Err(CodegenError::internal(format!(
                "a vtable needs at least {} slots, but {slots} were requested",
                seal_layout::VTABLE_HEADER_SLOTS
            )));
        }
        if let Some(found) = self.vtables.borrow().get(&slots).copied() {
            return Ok(found);
        }
        let mut fields = Vec::with_capacity(slots);
        fields.push(self.pointer().into());
        fields.push(self.usize_type()?.into());
        fields.push(self.usize_type()?.into());
        for _ in seal_layout::VTABLE_HEADER_SLOTS as usize..slots {
            fields.push(self.pointer().into());
        }
        let created = self.context.struct_type(&fields, false);
        self.vtables.borrow_mut().insert(slots, created);
        Ok(created)
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
    use inkwell::AddressSpace;
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
    fn a_dyn_pointer_is_two_pointer_words() {
        let context = Context::create();
        let tcx = TyCtxt::new();
        let interner = Interner::new();
        let trait_def = seal_def::DefId::from_usize_checked(0).expect("zero is a valid id");
        let object = tcx.dyn_ty(trait_def, tcx.empty_substs());
        let pointer = tcx.ref_ty(object);

        for pointer_width in [32, 64] {
            let mapper = TypeMapper::new(&context, &tcx, &interner, pointer_width)
                .expect("pointer width is supported");
            let fat = mapper
                .basic(pointer)
                .expect("a trait-object pointer is sized")
                .into_struct_type();
            let fields = fat.get_field_types();
            assert_eq!(fields.len(), 2);
            for field in fields {
                assert_eq!(
                    field.into_pointer_type().get_address_space(),
                    AddressSpace::default()
                );
            }
        }
    }

    #[test]
    fn vtable_types_are_cached_by_slot_count() {
        let context = Context::create();
        let tcx = TyCtxt::new();
        let interner = Interner::new();
        let mapper = TypeMapper::new(&context, &tcx, &interner, 64).expect("types map");

        let three = mapper.vtable_type(3).expect("header-only vtable exists");
        assert_eq!(three.get_field_types().len(), 3);
        assert_eq!(
            mapper.vtable_type(3).expect("cached").print_to_string(),
            three.print_to_string()
        );
        assert_eq!(
            mapper
                .vtable_type(5)
                .expect("two method slots exist")
                .get_field_types()
                .len(),
            5
        );
        assert!(mapper.vtable_type(2).is_err());
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
