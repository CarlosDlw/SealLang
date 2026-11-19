//! Vtable globals and coercion lookup.
//!
//! Every [`MonoVTable`] becomes one private, constant global. Slots follow the
//! language's [`VtableLayout`]: a no-op drop pointer, size, alignment, then the
//! concrete method pointers. Functions are declared before this module runs, so
//! every method slot resolves to an existing [`FunctionValue`].
//!
//! The lookup key is derived from a coercion's concrete source pointee and the
//! target trait object, reusing the same [`VtableKey`] monomorphization used to
//! collect the table. A missing key is an internal invariant violation, never a
//! null global that would crash at run time.

use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::values::{BasicValueEnum, FunctionValue, GlobalValue};
use seal_def::DefId;
use seal_hash::{FxHashMap, FxHashSet};
use seal_hir::Hir;
use seal_index::Idx;
use seal_mono::{MonoProgram, MonoVTable, VtableId, VtableKey};
use seal_ty::{Instance, TyCtxt, TyId};

use crate::error::CodegenError;
use crate::names;
use crate::types::TypeMapper;

/// The emitted vtable globals, addressable by coercion key.
pub(crate) struct Vtables<'ctx> {
    globals: FxHashMap<VtableId, GlobalValue<'ctx>>,
    ids: FxHashMap<VtableKey, VtableId>,
}

impl<'ctx> Vtables<'ctx> {
    /// Emits one no-op drop function and every collected vtable.
    ///
    /// `functions` must already contain every collected instance. A method
    /// entry without a declared function is reported as an internal error
    /// instead of leaving a null slot in a constant global.
    pub(crate) fn declare(
        context: &'ctx Context,
        module: &Module<'ctx>,
        hir: &Hir,
        mono: &MonoProgram,
        mapper: &TypeMapper<'ctx>,
        functions: &FxHashMap<Instance, FunctionValue<'ctx>>,
    ) -> Result<Self, CodegenError> {
        // A program without trait objects keeps exactly the IR it had before
        // vtable support: no header helper and no globals.
        if mono.vtables().is_empty() {
            return Ok(Self {
                globals: FxHashMap::default(),
                ids: FxHashMap::default(),
            });
        }

        // The header's drop slot shares the one glue entry every drop statement
        // calls, so the vtable shape stays stable once destructors exist.
        let drop = crate::drop::noop_drop(context, module)?;
        let mut globals = FxHashMap::default();
        let mut ids = FxHashMap::default();

        for (index, table) in mono.vtables().iter().enumerate() {
            let id = VtableId::from_usize(index);
            let global = emit_vtable(module, hir, mapper, functions, drop, table)?;
            globals.insert(id, global);
            ids.insert(table.key, id);
        }

        Ok(Self { globals, ids })
    }

    /// Resolves the vtable emitted for a coercion from `source_ty` to
    /// `target_ty`.
    ///
    /// This is the only call-site vtable lookup. It fails as an internal error
    /// when monomorphization did not collect the required table; emitting a
    /// null pointer instead would turn a compiler bug into a runtime crash.
    pub(crate) fn global_for_coercion(
        &self,
        tcx: &TyCtxt,
        source_ty: TyId,
        target_ty: TyId,
    ) -> Result<GlobalValue<'ctx>, CodegenError> {
        let key = VtableKey::from_coercion(tcx, source_ty, target_ty).ok_or_else(|| {
            CodegenError::internal(format!(
                "coercion from `{source_ty:?}` to `{target_ty:?}` is not a trait-object conversion"
            ))
        })?;
        self.global_for_key(key)
    }

    /// Resolves the vtable collected for `key`.
    ///
    /// Constant trait objects name their table by the concrete type and trait
    /// reference recorded in the value, which is the same key a coercion
    /// produces, so both paths share one lookup.
    pub(crate) fn global_for_key(&self, key: VtableKey) -> Result<GlobalValue<'ctx>, CodegenError> {
        let id = self.ids.get(&key).copied().ok_or_else(|| {
            CodegenError::internal(format!(
                "monomorphization did not collect a vtable for `{key:?}`"
            ))
        })?;
        self.globals.get(&id).copied().ok_or_else(|| {
            CodegenError::internal(format!("vtable {id:?} was collected but never emitted"))
        })
    }
}

/// Emits one private constant vtable global.
fn emit_vtable<'ctx>(
    module: &Module<'ctx>,
    hir: &Hir,
    mapper: &TypeMapper<'ctx>,
    functions: &FxHashMap<Instance, FunctionValue<'ctx>>,
    drop: FunctionValue<'ctx>,
    table: &MonoVTable,
) -> Result<GlobalValue<'ctx>, CodegenError> {
    let slot_count = table.entries.len() + seal_layout::VTABLE_HEADER_SLOTS as usize;
    let vtable_type = mapper.vtable_type(slot_count)?;
    let layout = mapper.layout(table.key.concrete)?;

    let mut fields: Vec<BasicValueEnum<'ctx>> = Vec::with_capacity(slot_count);
    fields.push(drop.as_global_value().as_pointer_value().into());
    fields.push(mapper.usize_type()?.const_int(layout.size(), false).into());
    fields.push(mapper.usize_type()?.const_int(layout.align(), false).into());
    for entry in &table.entries {
        let function = functions.get(entry).copied().ok_or_else(|| {
            CodegenError::internal(format!(
                "vtable for `{:?}` method {entry:?} was not declared as a function",
                table.key
            ))
        })?;
        fields.push(function.as_global_value().as_pointer_value().into());
    }

    let initializer = vtable_type.const_named_struct(&fields);
    let name = names::vtable(seal_mono::vtable_hash(hir, &table.key));
    let global = module.add_global(vtable_type, None, &name);
    global.set_initializer(&initializer);
    global.set_constant(true);
    global.set_linkage(Linkage::Private);
    global.set_unnamed_addr(true);
    global.set_alignment(u32::from(mapper.pointer_width()) / 8);
    Ok(global)
}

/// Returns a copy of `facts` without any function reachable through a vtable.
///
/// A dynamic call is a call site the whole-program aliasing proof never saw, so
/// an exclusive receiver of a vtable method cannot be marked `noalias`. This
/// includes supertrait methods, because they share the same vtable. The input
/// set is never mutated.
pub(crate) fn filtered_noalias(mono: &MonoProgram, facts: &FxHashSet<DefId>) -> FxHashSet<DefId> {
    let mut filtered = facts.clone();
    for table in mono.vtables().iter() {
        for entry in &table.entries {
            filtered.remove(&entry.def);
        }
        if let Some(drop) = table.drop {
            filtered.remove(&drop.def);
        }
    }
    filtered
}
