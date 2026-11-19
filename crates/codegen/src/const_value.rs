//! Lowering evaluated constants to memory images.
//!
//! Every constant is encoded from its evaluated value into its exact memory
//! image: a byte buffer with one symbolic pointer slot per address it contains.
//! Encoding follows the language layout at every level, so padding, enum tags,
//! zero-sized values, and nested pointers all land at the byte the rest of the
//! compiler expects.
//!
//! Turning an image into an LLVM type and initializer is [`crate::const_image`]'s
//! job, and resolving the pointer slots is [`crate::relocation`]'s.

use std::cell::RefCell;

use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::types::BasicTypeEnum;
use inkwell::values::GlobalValue;
use seal_const_eval::{ConstProgram, ConstValue};
use seal_def::DefId;
use seal_hash::FxHashMap;
use seal_hir::Hir;
use seal_symbol::Interner;
use seal_target::Endianness;
use seal_ty::{TyCtxt, TyId, TyKind};

use crate::const_image::Image;
use crate::error::CodegenError;
use crate::names;
use crate::relocation::{PointerTarget, Pointers};
use crate::types::TypeMapper;

/// Encodes evaluated constants for one module.
///
/// The encoder owns the cache of constant values read by `Operand::Const`: a
/// value has no address in the language, so it is read from one private,
/// constant global that is created on first use and never mutated.
pub(crate) struct ConstValues<'a, 'ctx> {
    context: &'ctx Context,
    hir: &'a Hir,
    mapper: &'a TypeMapper<'ctx>,
    program: &'a ConstProgram,
    interner: &'a Interner,
    endianness: Endianness,
    operands: RefCell<FxHashMap<DefId, GlobalValue<'ctx>>>,
}

impl<'a, 'ctx> ConstValues<'a, 'ctx> {
    /// Creates an encoder for one module.
    pub(crate) fn new(
        context: &'ctx Context,
        hir: &'a Hir,
        mapper: &'a TypeMapper<'ctx>,
        program: &'a ConstProgram,
        interner: &'a Interner,
        endianness: Endianness,
    ) -> Self {
        Self {
            context,
            hir,
            mapper,
            program,
            interner,
            endianness,
            operands: RefCell::new(FxHashMap::default()),
        }
    }

    /// Encodes `value`, declared as `ty`, into its memory image.
    pub(crate) fn encode(&self, ty: TyId, value: &ConstValue) -> Result<Image, CodegenError> {
        let size = self.mapper.layout(ty)?.size();
        let mut image = Image::new(size, self.endianness, self.pointer_size());
        self.encode_value(value, ty, 0, &mut image)?;
        Ok(image)
    }

    /// Returns the private global holding the evaluated value of `def`.
    ///
    /// The global is immutable and module-local. A value global is created only
    /// for constants that a body reads by value; addressed constants use their
    /// allocation global instead.
    pub(crate) fn value_global(
        &self,
        module: &Module<'ctx>,
        def: DefId,
        pointers: &Pointers<'_, 'ctx>,
    ) -> Result<GlobalValue<'ctx>, CodegenError> {
        if let Some(found) = self.operands.borrow().get(&def).copied() {
            return Ok(found);
        }
        let ty = self
            .hir
            .body_of(def)
            .and_then(|body| body.ret)
            .ok_or_else(|| {
                CodegenError::internal(format!(
                    "constant {def:?} has no declared type in its HIR body"
                ))
            })?;
        let value = self.program.value(def).ok_or_else(|| {
            CodegenError::internal(format!(
                "constant {def:?} is read by value but was never evaluated"
            ))
        })?;
        let (name, parent) = names::definition(self.hir, self.interner, def);
        let symbol = names::value(&name, parent.as_deref());
        // The value is created on first use, which is after every program
        // global exists, so one pass suffices: nothing can point at this copy.
        let image = self.encode(ty, value)?;
        let global = module.add_global(self.image_type(&image)?, None, &symbol);
        global.set_linkage(Linkage::Private);
        global.set_constant(true);
        global.set_alignment(self.alignment(ty)?);
        self.set_initializer(global, &image, pointers)?;
        self.operands.borrow_mut().insert(def, global);
        Ok(global)
    }

    /// Returns the LLVM type of an encoded image.
    pub(crate) fn image_type(&self, image: &Image) -> Result<BasicTypeEnum<'ctx>, CodegenError> {
        image.llvm_type(self.context)
    }

    /// Sets the initializer of a declared global from an encoded image.
    pub(crate) fn set_initializer(
        &self,
        global: GlobalValue<'ctx>,
        image: &Image,
        pointers: &Pointers<'_, 'ctx>,
    ) -> Result<(), CodegenError> {
        let initializer = image.llvm_value(self.context, pointers)?;
        global.set_initializer(&initializer);
        Ok(())
    }

    /// Returns the LLVM alignment of `ty`, never zero.
    ///
    /// LLVM rejects a zero alignment, and a zero-sized value still occupies one
    /// byte of backing storage so its address is unique.
    pub(crate) fn alignment(&self, ty: TyId) -> Result<u32, CodegenError> {
        let align = self.mapper.layout(ty)?.align().max(1);
        u32::try_from(align).map_err(|_| {
            CodegenError::internal(format!("type {ty:?} has an alignment too large for LLVM"))
        })
    }

    fn tcx(&self) -> &TyCtxt {
        self.hir.ty_ctxt()
    }

    fn size(&self, ty: TyId) -> Result<u64, CodegenError> {
        Ok(self.mapper.layout(ty)?.size())
    }

    fn pointer_size(&self) -> u64 {
        u64::from(self.mapper.pointer_width()) / 8
    }

    /// Requires that `ty` occupies exactly `expected` bytes.
    ///
    /// The check keeps the encoder tied to the type checker: a value whose
    /// declared size disagrees with the destination slot would silently corrupt
    /// the image instead of failing loudly.
    fn expect_size(&self, ty: TyId, expected: u64) -> Result<(), CodegenError> {
        let size = self.size(ty)?;
        if size != expected {
            return Err(CodegenError::internal(format!(
                "constant slot of type {ty:?} is {size} bytes, but the value needs {expected}"
            )));
        }
        Ok(())
    }

    fn encode_value(
        &self,
        value: &ConstValue,
        ty: TyId,
        base: u64,
        image: &mut Image,
    ) -> Result<(), CodegenError> {
        match value {
            ConstValue::Int { bits, ty: value_ty } => {
                let size = self.size(*value_ty)?;
                self.expect_size(ty, size)?;
                image.write(base, size, *bits)?;
            }
            ConstValue::Float { bits, ty: value_ty } => {
                let size = self.size(*value_ty)?;
                self.expect_size(ty, size)?;
                image.write(base, size, u128::from(*bits))?;
            }
            ConstValue::Bool(flag) => {
                self.expect_size(ty, 1)?;
                image.write(base, 1, u128::from(*flag))?;
            }
            ConstValue::Char(scalar) => {
                self.expect_size(ty, 4)?;
                image.write(base, 4, u128::from(u32::from(*scalar)))?;
            }
            ConstValue::Unit => self.expect_size(ty, 0)?,
            ConstValue::Tuple(elements) => {
                let TyKind::Tuple(tuple) = self.tcx().kind(ty) else {
                    return Err(self.mismatch("tuple", ty));
                };
                let layouts = self.mapper.tuple_field_layouts(ty)?;
                if elements.len() != layouts.len() {
                    return Err(CodegenError::internal(format!(
                        "tuple constant has {} elements for a tuple of {}",
                        elements.len(),
                        layouts.len()
                    )));
                }
                for (index, element) in elements.iter().enumerate() {
                    let element_ty = self.tcx().tuple_element(tuple, index).ok_or_else(|| {
                        CodegenError::internal(format!(
                            "tuple element {index} is outside the tuple of {ty:?}"
                        ))
                    })?;
                    self.encode_value(element, element_ty, base + layouts[index].offset, image)?;
                }
            }
            ConstValue::Array(elements) => {
                let TyKind::Array { element, length } = self.tcx().kind(ty) else {
                    return Err(self.mismatch("array", ty));
                };
                if elements.len() as u64 != length {
                    return Err(CodegenError::internal(format!(
                        "array constant has {} elements for a length of {length}",
                        elements.len()
                    )));
                }
                let element_size = self.size(element)?;
                for (index, value) in elements.iter().enumerate() {
                    let offset = base + index as u64 * element_size;
                    self.encode_value(value, element, offset, image)?;
                }
            }
            ConstValue::Struct { adt, args, fields } => {
                let adt_id = self.adt_id(*adt)?;
                let TyKind::Adt {
                    adt: ty_adt,
                    args: ty_args,
                } = self.tcx().kind(ty)
                else {
                    return Err(self.mismatch("struct", ty));
                };
                if ty_adt != adt_id || ty_args != *args {
                    return Err(CodegenError::internal(format!(
                        "struct constant for {adt:?} does not match its declared type {ty:?}"
                    )));
                }
                let definition = self.tcx().adt(adt_id);
                if fields.len() != definition.fields.len() {
                    return Err(CodegenError::internal(format!(
                        "struct constant for {adt:?} has {} fields for {} declarations",
                        fields.len(),
                        definition.fields.len()
                    )));
                }
                let layouts = self.mapper.field_layouts(adt_id, *args)?;
                for (index, (field, (field_def, value))) in
                    definition.fields.iter().zip(fields).enumerate()
                {
                    if field.def != *field_def {
                        return Err(CodegenError::internal(format!(
                            "struct constant field {field_def:?} is out of declaration order in {adt:?}"
                        )));
                    }
                    let field_ty = self.tcx().substitute(field.ty, *args);
                    self.encode_value(value, field_ty, base + layouts[index].offset, image)?;
                }
            }
            ConstValue::Variant {
                adt,
                variant,
                args,
                fields,
            } => {
                let adt_id = self.adt_id(*adt)?;
                let layout = self.mapper.enum_layout(adt_id, *args)?;
                image.write(base, layout.tag.size(), u128::from(variant.index()))?;
                let definition = self.tcx().adt(adt_id);
                let variant_def = definition.variant(*variant).ok_or_else(|| {
                    CodegenError::internal(format!(
                        "variant {variant:?} does not belong to enum {adt:?}"
                    ))
                })?;
                if fields.len() != variant_def.fields.len() {
                    return Err(CodegenError::internal(format!(
                        "variant constant for {adt:?} has {} fields for {} declarations",
                        fields.len(),
                        variant_def.fields.len()
                    )));
                }
                let variant_layout =
                    layout
                        .variants
                        .get(variant.index() as usize)
                        .ok_or_else(|| {
                            CodegenError::internal(format!(
                                "enum {adt:?} has no layout for {variant:?}"
                            ))
                        })?;
                for (index, (field, (field_def, value))) in
                    variant_def.fields.iter().zip(fields).enumerate()
                {
                    if field.def != *field_def {
                        return Err(CodegenError::internal(format!(
                            "variant constant field {field_def:?} is out of declaration order in {adt:?}"
                        )));
                    }
                    let field_layout = variant_layout.fields.get(index).ok_or_else(|| {
                        CodegenError::internal(format!(
                            "enum {adt:?} has no layout for field {field_def:?}"
                        ))
                    })?;
                    let field_ty = self.tcx().substitute(field.ty, *args);
                    self.encode_value(value, field_ty, base + field_layout.offset, image)?;
                }
            }
            ConstValue::Str { value: text, .. } => {
                let pointer_size = self.pointer_size();
                self.expect_size(ty, pointer_size * 2)?;
                image.slot(
                    base,
                    PointerTarget::String {
                        symbol: *text,
                        offset: 0,
                    },
                )?;
                let length = self.interner.resolve(*text).len() as u128;
                image.write(base + pointer_size, pointer_size, length)?;
            }
            ConstValue::Ref { target, offset, .. } => {
                let pointer_size = self.pointer_size();
                self.expect_size(ty, pointer_size)?;
                image.slot(
                    base,
                    PointerTarget::Allocation {
                        id: *target,
                        offset: *offset,
                    },
                )?;
            }
            ConstValue::Slice {
                target,
                offset,
                length,
                ..
            } => {
                let pointer_size = self.pointer_size();
                self.expect_size(ty, pointer_size * 2)?;
                image.slot(
                    base,
                    PointerTarget::Allocation {
                        id: *target,
                        offset: *offset,
                    },
                )?;
                image.write(base + pointer_size, pointer_size, u128::from(*length))?;
            }
            ConstValue::Dyn {
                data,
                data_offset,
                concrete,
                trait_ref,
            }
            | ConstValue::RawDyn {
                data,
                data_offset,
                concrete,
                trait_ref,
                ..
            } => {
                let pointer_size = self.pointer_size();
                self.expect_size(ty, pointer_size * 2)?;
                image.slot(
                    base,
                    PointerTarget::Allocation {
                        id: *data,
                        offset: *data_offset,
                    },
                )?;
                image.slot(
                    base + pointer_size,
                    PointerTarget::Vtable {
                        concrete: *concrete,
                        trait_ref: *trait_ref,
                    },
                )?;
            }
            ConstValue::RawPtr { target, offset, .. } => {
                let pointer_size = self.pointer_size();
                self.expect_size(ty, pointer_size)?;
                image.slot(
                    base,
                    PointerTarget::Allocation {
                        id: *target,
                        offset: *offset,
                    },
                )?;
            }
            ConstValue::RawSlice {
                target,
                offset,
                length,
                ..
            } => {
                let pointer_size = self.pointer_size();
                self.expect_size(ty, pointer_size * 2)?;
                image.slot(
                    base,
                    PointerTarget::Allocation {
                        id: *target,
                        offset: *offset,
                    },
                )?;
                image.write(base + pointer_size, pointer_size, u128::from(*length))?;
            }
            ConstValue::RawStr { value: text, .. } => {
                let pointer_size = self.pointer_size();
                self.expect_size(ty, pointer_size * 2)?;
                image.slot(
                    base,
                    PointerTarget::String {
                        symbol: *text,
                        offset: 0,
                    },
                )?;
                let length = self.interner.resolve(*text).len() as u128;
                image.write(base + pointer_size, pointer_size, length)?;
            }
            ConstValue::RawStrData { value, offset, .. } => {
                let pointer_size = self.pointer_size();
                self.expect_size(ty, pointer_size)?;
                image.slot(
                    base,
                    PointerTarget::String {
                        symbol: *value,
                        offset: *offset,
                    },
                )?;
            }
        }
        Ok(())
    }

    fn adt_id(&self, def: DefId) -> Result<seal_ty::AdtId, CodegenError> {
        self.hir.adt(def).ok_or_else(|| {
            CodegenError::internal(format!("{def:?} is not an aggregate definition"))
        })
    }

    fn mismatch(&self, expected: &str, ty: TyId) -> CodegenError {
        CodegenError::internal(format!(
            "a {expected} constant is encoded into the type {ty:?}"
        ))
    }
}
