//! Drop glue functions.
//!
//! Dropping a value runs one glue function per concrete type. A glue takes the
//! erased data pointer, calls the type's destructor when it declares one, and
//! then drops its children: the fields of a struct in declaration order, the
//! active variant's fields of an enum, or every element of an array.
//!
//! The function is internal and keeps the vtable's first-slot signature, so a
//! trait object can name its concrete type's glue without an adapter. Glue is
//! declared before vtables are initialized, because a vtable slot points at it,
//! and filled before any body is lowered, because a drop statement calls it.

mod adt;
mod sequence;

use inkwell::AddressSpace;
use inkwell::builder::{Builder, BuilderError};
use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::values::{FunctionValue, IntValue, PointerValue};
use seal_hash::FxHashMap;
use seal_hir::Hir;
use seal_llvm_ffi::gep;
use seal_mono::{DropGlueEntry, GlueChildren, MonoProgram};
use seal_ty::{Instance, TyId, TyKind};

use crate::error::CodegenError;
use crate::names;
use crate::types::TypeMapper;

/// The declared drop glue functions, keyed by concrete type.
pub(crate) struct DropGlues<'ctx> {
    functions: FxHashMap<TyId, FunctionValue<'ctx>>,
}

impl<'ctx> DropGlues<'ctx> {
    /// Declares one internal `void (ptr)` per collected glue.
    ///
    /// The symbol carries the stable type fingerprint, so two types never share
    /// a glue and repeated builds emit the same names.
    pub(crate) fn declare(
        context: &'ctx Context,
        module: &Module<'ctx>,
        hir: &Hir,
        mono: &MonoProgram,
    ) -> Result<Self, CodegenError> {
        let pointer = context.ptr_type(AddressSpace::default());
        let function_type = context.void_type().fn_type(&[pointer.into()], false);
        let mut functions = FxHashMap::default();
        for entry in mono.drop_glues().iter() {
            let name = names::drop_glue(seal_mono::type_hash(hir, entry.ty));
            let function = module.add_function(&name, function_type, Some(Linkage::Internal));
            functions.insert(entry.ty, function);
        }
        Ok(Self { functions })
    }

    /// Returns the declared glue of `ty`.
    ///
    /// Monomorphization collects one glue per type a drop site or a vtable
    /// names, so a missing function is an internal invariant violation rather
    /// than something a user program can cause.
    pub(crate) fn function_for(&self, ty: TyId) -> Result<FunctionValue<'ctx>, CodegenError> {
        self.functions.get(&ty).copied().ok_or_else(|| {
            CodegenError::internal(format!(
                "monomorphization collected no drop glue for `{ty:?}`"
            ))
        })
    }

    /// Fills every declared glue with its destructor call and children.
    pub(crate) fn emit(
        &self,
        context: &'ctx Context,
        hir: &Hir,
        mapper: &TypeMapper<'ctx>,
        mono: &MonoProgram,
        functions: &FxHashMap<Instance, FunctionValue<'ctx>>,
    ) -> Result<(), CodegenError> {
        let builder: Builder<'ctx> = context.create_builder();
        for entry in mono.drop_glues().iter() {
            let function = self.function_for(entry.ty)?;
            let block = context.append_basic_block(function, "entry");
            builder.position_at_end(block);
            GlueBuilder {
                context,
                function,
                builder: &builder,
                hir,
                mapper,
                functions,
                glues: self,
            }
            .emit_entry(entry)?;
        }
        Ok(())
    }
}

/// Emits one glue function body.
struct GlueBuilder<'a, 'ctx> {
    context: &'ctx Context,
    function: FunctionValue<'ctx>,
    builder: &'a Builder<'ctx>,
    hir: &'a Hir,
    mapper: &'a TypeMapper<'ctx>,
    functions: &'a FxHashMap<Instance, FunctionValue<'ctx>>,
    glues: &'a DropGlues<'ctx>,
}

impl<'a, 'ctx> GlueBuilder<'a, 'ctx> {
    /// Emits the destructor call and the children of one glue.
    fn emit_entry(&self, entry: &DropGlueEntry) -> Result<(), CodegenError> {
        let data = self.data(entry.ty)?;
        if let Some(destructor) = entry.glue.destructor {
            let function = self.functions.get(&destructor).copied().ok_or_else(|| {
                CodegenError::internal(format!(
                    "the destructor of `{:?}` was never declared as a function",
                    entry.ty
                ))
            })?;
            self.call(function, data)?;
        }
        match &entry.glue.children {
            GlueChildren::None => self.ret(),
            GlueChildren::Fields(fields) => {
                match self.hir.ty_ctxt().kind(entry.ty) {
                    TyKind::Adt { .. } => self.emit_fields(entry.ty, fields)?,
                    TyKind::Tuple(_) => self.emit_tuple(entry.ty, fields)?,
                    other => {
                        return Err(CodegenError::internal(format!(
                            "the drop glue of `{:?}` lists fields for `{other:?}`",
                            entry.ty
                        )));
                    }
                }
                self.ret()
            }
            // Every variant path returns or panics on its own, including the
            // default arm, so the switch never falls through to a shared return.
            GlueChildren::Variants(variants) => self.emit_variants(entry.ty, variants),
            GlueChildren::Elements { element, count } => {
                self.emit_elements(*element, *count)?;
                self.ret()
            }
        }
    }

    /// Returns the glue's data pointer.
    fn data(&self, ty: TyId) -> Result<PointerValue<'ctx>, CodegenError> {
        self.function
            .get_nth_param(0)
            .map(|parameter| parameter.into_pointer_value())
            .ok_or_else(|| {
                CodegenError::internal(format!("the drop glue of `{ty:?}` has no data parameter"))
            })
    }

    fn ret(&self) -> Result<(), CodegenError> {
        self.build(self.builder.build_return(None), "drop glue return")
            .map(|_| ())
    }

    fn call(
        &self,
        callee: FunctionValue<'ctx>,
        data: PointerValue<'ctx>,
    ) -> Result<(), CodegenError> {
        self.build(
            self.builder.build_call(callee, &[data.into()], ""),
            "drop glue call",
        )
        .map(|_| ())
    }

    /// Returns the address `offset` bytes past `pointer`.
    ///
    /// Enum variant fields live at absolute offsets from the enum value, given
    /// by the language layout, so the GEP stays inside the enum allocation.
    fn offset(
        &self,
        pointer: PointerValue<'ctx>,
        offset: u64,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let offset = self.mapper.usize_type()?.const_int(offset, false);
        self.build(
            gep(
                self.builder,
                pointer,
                self.context.i8_type(),
                &[offset],
                true,
            ),
            "field pointer",
        )
    }

    /// Returns the address of `index` elements into an array.
    ///
    /// The array has a known layout and MIR checks the index, so the GEP is
    /// inbounds and computes the stride from the element type.
    fn index(
        &self,
        pointer: PointerValue<'ctx>,
        index: IntValue<'ctx>,
        element: TyId,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let element_type = self.mapper.basic(element)?;
        self.build(
            gep(self.builder, pointer, element_type, &[index], true),
            "array element address",
        )
    }

    fn build<T>(
        &self,
        result: Result<T, BuilderError>,
        operation: &str,
    ) -> Result<T, CodegenError> {
        result
            .map_err(|error| CodegenError::internal(format!("cannot build {operation}: {error:?}")))
    }
}
