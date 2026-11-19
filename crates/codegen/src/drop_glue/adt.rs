//! Struct and enum drop glue bodies.
//!
//! A struct drops its droppable fields in declaration order, so the LLVM struct
//! index is the field's declaration index. An enum drops only the fields of its
//! active variant: the tag selects one block per variant, and each field is
//! reached through its absolute offset in the language layout.

use inkwell::basic_block::BasicBlock;
use inkwell::values::IntValue;
use seal_index::Idx;
use seal_ty::{SubstsId, TyId, TyKind, VariantIdx};

use super::GlueBuilder;
use crate::error::CodegenError;

impl<'ctx> GlueBuilder<'_, 'ctx> {
    /// Emits the field drops of a struct glue.
    pub(super) fn emit_fields(&self, ty: TyId, children: &[TyId]) -> Result<(), CodegenError> {
        let tcx = self.hir.ty_ctxt();
        let TyKind::Adt { adt, args } = tcx.kind(ty) else {
            return Err(CodegenError::internal(format!(
                "a struct drop glue names `{ty:?}`"
            )));
        };
        let definition = tcx.adt(adt);
        let struct_type = self.mapper.adt(adt, args)?;
        let data = self.data(ty)?;
        let mut remaining = children.iter().copied();
        for (index, field) in definition.fields.iter().enumerate() {
            let field_ty = tcx.substitute(field.ty, args);
            if !tcx.needs_drop(field_ty) {
                continue;
            }
            let child = remaining.next().ok_or_else(|| {
                CodegenError::internal(format!(
                    "the drop glue of `{ty:?}` lists fewer fields than the struct declares"
                ))
            })?;
            if child != field_ty {
                return Err(CodegenError::internal(format!(
                    "the drop glue of `{ty:?}` names `{child:?}` where field {index} is `{field_ty:?}`"
                )));
            }
            let pointer = self.build(
                self.builder
                    .build_struct_gep(struct_type, data, index as u32, "field"),
                "field projection",
            )?;
            self.call(self.glues.function_for(child)?, pointer)?;
        }
        if remaining.next().is_some() {
            return Err(CodegenError::internal(format!(
                "the drop glue of `{ty:?}` lists more fields than the struct drops"
            )));
        }
        Ok(())
    }

    /// Emits the variant switch and the field drops of an enum glue.
    ///
    /// Each variant block returns on its own, and the tag's default arm is
    /// unreachable: an enum value always carries a declared discriminant.
    pub(super) fn emit_variants(
        &self,
        ty: TyId,
        variants: &[Vec<TyId>],
    ) -> Result<(), CodegenError> {
        let tcx = self.hir.ty_ctxt();
        let TyKind::Adt { adt, args } = tcx.kind(ty) else {
            return Err(CodegenError::internal(format!(
                "an enum drop glue names `{ty:?}`"
            )));
        };
        let definition = tcx.adt(adt);
        if definition.variants.len() != variants.len() {
            return Err(CodegenError::internal(format!(
                "the drop glue of `{ty:?}` lists {} variants, but the enum declares {}",
                variants.len(),
                definition.variants.len()
            )));
        }
        let layout = self.mapper.enum_layout(adt, args)?;
        let enum_type = self.mapper.adt(adt, args)?;
        let data = self.data(ty)?;
        let tag_type = self.tag_type(ty, layout.tag.size())?;
        let tag_pointer = self.build(
            self.builder
                .build_struct_gep(enum_type, data, 0, "enum.tag"),
            "enum tag projection",
        )?;
        let tag = self
            .build(
                self.builder.build_load(tag_type, tag_pointer, "enum tag"),
                "enum tag load",
            )?
            .into_int_value();
        let otherwise = self.context.append_basic_block(self.function, "drop.other");
        let blocks: Vec<BasicBlock<'ctx>> = variants
            .iter()
            .enumerate()
            .map(|(index, _)| {
                self.context
                    .append_basic_block(self.function, &format!("drop.variant{index}"))
            })
            .collect();
        let cases: Vec<(IntValue<'ctx>, BasicBlock<'ctx>)> = blocks
            .iter()
            .enumerate()
            .map(|(index, block)| (tag_type.const_int(index as u64, false), *block))
            .collect();
        self.build(
            self.builder.build_switch(tag, otherwise, &cases),
            "enum tag switch",
        )?;
        self.builder.position_at_end(otherwise);
        self.build(self.builder.build_unreachable(), "unreachable variant")?;

        for (index, (children, block)) in variants.iter().zip(&blocks).enumerate() {
            self.builder.position_at_end(*block);
            self.emit_variant_fields(ty, adt, args, VariantIdx::from_usize(index), children)?;
            self.ret()?;
        }
        Ok(())
    }

    /// Emits the field drops of one variant block.
    fn emit_variant_fields(
        &self,
        ty: TyId,
        adt: seal_ty::AdtId,
        args: SubstsId,
        variant: VariantIdx,
        children: &[TyId],
    ) -> Result<(), CodegenError> {
        let tcx = self.hir.ty_ctxt();
        let definition = tcx.adt(adt);
        let variant_definition = definition.variant(variant).ok_or_else(|| {
            CodegenError::internal(format!(
                "the drop glue of `{ty:?}` names variant {variant:?}, which does not exist"
            ))
        })?;
        let layout = self.mapper.enum_layout(adt, args)?;
        let data = self.data(ty)?;
        let mut remaining = children.iter().copied();
        for (index, field) in variant_definition.fields.iter().enumerate() {
            let field_ty = tcx.substitute(field.ty, args);
            if !tcx.needs_drop(field_ty) {
                continue;
            }
            let child = remaining.next().ok_or_else(|| {
                CodegenError::internal(format!(
                    "the drop glue of `{ty:?}` lists fewer fields than variant {variant:?} declares"
                ))
            })?;
            if child != field_ty {
                return Err(CodegenError::internal(format!(
                    "the drop glue of `{ty:?}` names `{child:?}` where variant {variant:?} field {index} is `{field_ty:?}`"
                )));
            }
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
            let pointer = self.offset(data, offset)?;
            self.call(self.glues.function_for(child)?, pointer)?;
        }
        if remaining.next().is_some() {
            return Err(CodegenError::internal(format!(
                "the drop glue of `{ty:?}` lists more fields than variant {variant:?} drops"
            )));
        }
        Ok(())
    }

    /// Returns the integer type of an enum tag.
    fn tag_type(&self, ty: TyId, size: u64) -> Result<inkwell::types::IntType<'ctx>, CodegenError> {
        let bits = u16::try_from(size * 8).map_err(|_| {
            CodegenError::internal(format!(
                "the drop glue of `{ty:?}` has a tag too wide for codegen"
            ))
        })?;
        self.mapper.integer_type(bits)
    }
}
