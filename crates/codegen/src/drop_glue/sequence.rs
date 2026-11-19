//! Tuple and array drop glue bodies.
//!
//! A tuple drops its droppable elements in order, one LLVM struct field each.
//! An array drops every element in a loop, so the glue stays one function no
//! matter how many elements the type has.

use inkwell::IntPredicate;
use seal_ty::{TyId, TyKind};

use super::GlueBuilder;
use crate::error::CodegenError;

impl GlueBuilder<'_, '_> {
    /// Emits the element drops of a tuple glue.
    pub(super) fn emit_tuple(&self, ty: TyId, children: &[TyId]) -> Result<(), CodegenError> {
        let tcx = self.hir.ty_ctxt();
        let TyKind::Tuple(tuple) = tcx.kind(ty) else {
            return Err(CodegenError::internal(format!(
                "a tuple drop glue names `{ty:?}`"
            )));
        };
        let elements = tcx.tuple_elements(tuple);
        let tuple_type = self.mapper.basic(ty)?.into_struct_type();
        let data = self.data(ty)?;
        let mut remaining = children.iter().copied();
        for (index, element) in elements.iter().enumerate() {
            if !tcx.needs_drop(*element) {
                continue;
            }
            let child = remaining.next().ok_or_else(|| {
                CodegenError::internal(format!(
                    "the drop glue of `{ty:?}` lists fewer elements than the tuple declares"
                ))
            })?;
            if child != *element {
                return Err(CodegenError::internal(format!(
                    "the drop glue of `{ty:?}` names `{child:?}` where element {index} is `{element:?}`"
                )));
            }
            let pointer = self.build(
                self.builder
                    .build_struct_gep(tuple_type, data, index as u32, "tuple.element"),
                "tuple element projection",
            )?;
            self.call(self.glues.function_for(child)?, pointer)?;
        }
        if remaining.next().is_some() {
            return Err(CodegenError::internal(format!(
                "the drop glue of `{ty:?}` lists more elements than the tuple drops"
            )));
        }
        Ok(())
    }

    /// Emits a loop that drops `count` elements of `element`.
    pub(super) fn emit_elements(&self, element: TyId, count: u64) -> Result<(), CodegenError> {
        if count == 0 {
            return Ok(());
        }
        let glue = self.glues.function_for(element)?;
        let usize_type = self.mapper.usize_type()?;
        let data = self.data(element)?;
        let index = self.build(
            self.builder.build_alloca(usize_type, "drop.index"),
            "drop index storage",
        )?;
        self.build(
            self.builder.build_store(index, usize_type.const_zero()),
            "drop index store",
        )?;
        let check = self.context.append_basic_block(self.function, "drop.check");
        let body = self.context.append_basic_block(self.function, "drop.body");
        let done = self.context.append_basic_block(self.function, "drop.done");
        self.build(
            self.builder.build_unconditional_branch(check),
            "drop branch",
        )?;

        self.builder.position_at_end(check);
        let current = self
            .build(
                self.builder.build_load(usize_type, index, "drop.index"),
                "drop index load",
            )?
            .into_int_value();
        let keep = self.build(
            self.builder.build_int_compare(
                IntPredicate::ULT,
                current,
                usize_type.const_int(count, false),
                "drop.keep",
            ),
            "drop comparison",
        )?;
        self.build(
            self.builder.build_conditional_branch(keep, body, done),
            "drop loop branch",
        )?;

        self.builder.position_at_end(body);
        let element_pointer = self.index(data, current, element)?;
        self.call(glue, element_pointer)?;
        let next = self.build(
            self.builder
                .build_int_add(current, usize_type.const_int(1, false), "drop.next"),
            "drop index",
        )?;
        self.build(self.builder.build_store(index, next), "drop index store")?;
        self.build(
            self.builder.build_unconditional_branch(check),
            "drop loop branch",
        )?;

        self.builder.position_at_end(done);
        Ok(())
    }
}
