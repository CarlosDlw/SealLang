//! Program-lifetime globals: statics and addressed constants.
//!
//! Every reachable global is declared first and initialized afterwards. Two
//! passes are what let a static refer to a static declared later and let an
//! allocation refer to itself: the address of a declared global is valid before
//! its initializer is known.
//!
//! An immutable static is a `constant` global, a mutable static is writable, and
//! both keep internal linkage and the exact language alignment. The linker
//! places a constant global in read-only data and a writable one in data, so no
//! explicit section is needed. Const allocations are private and constant: they
//! are only reachable through a reference the program already holds.
//!
//! A zero-sized global receives one byte of backing storage. The byte is never
//! read or written; it exists so the global has a unique address, which is the
//! only observable property of a zero-sized static.

use inkwell::module::{Linkage, Module};
use inkwell::values::{GlobalValue, PointerValue};
use seal_const_eval::{AllocId, AllocSource, ConstProgram, ConstValue};
use seal_def::DefId;
use seal_hash::FxHashMap;
use seal_hir::Hir;
use seal_mono::MonoProgram;
use seal_symbol::Interner;
use seal_ty::TyId;

use crate::const_image::Image;
use crate::const_value::ConstValues;
use crate::error::CodegenError;
use crate::names;
use crate::relocation::Pointers;

/// One static's emitted storage.
struct StaticGlobal<'ctx> {
    global: GlobalValue<'ctx>,
    ty: TyId,
}

/// Every global emitted for one module.
pub(crate) struct Globals<'ctx> {
    statics: FxHashMap<DefId, StaticGlobal<'ctx>>,
    /// Declaration order. Iteration must not depend on hash order, or two runs
    /// of the same program would attach debug records in a different order.
    order: Vec<DefId>,
    allocations: FxHashMap<AllocId, GlobalValue<'ctx>>,
    initializers: Vec<(GlobalValue<'ctx>, Image)>,
}

impl<'ctx> Globals<'ctx> {
    /// Declares every reachable static and const allocation without filling it.
    ///
    /// `values` encodes each initializer, which also determines the LLVM type of
    /// the global. Callers must run [`Globals::initialize`] before verification.
    pub(crate) fn declare(
        module: &Module<'ctx>,
        hir: &Hir,
        mono: &MonoProgram,
        program: &ConstProgram,
        values: &ConstValues<'_, 'ctx>,
        interner: &Interner,
    ) -> Result<Self, CodegenError> {
        let mut globals = Self {
            statics: FxHashMap::default(),
            order: Vec::new(),
            allocations: FxHashMap::default(),
            initializers: Vec::new(),
        };

        for def in mono.statics() {
            let data = program.static_data(*def).copied().ok_or_else(|| {
                CodegenError::internal(format!(
                    "static {def:?} is reachable but constant evaluation produced no storage"
                ))
            })?;
            let allocation = program.allocation(data.alloc);
            if allocation.ty != data.ty {
                return Err(CodegenError::internal(format!(
                    "static {def:?} declares {:?} but its allocation holds {:?}",
                    data.ty, allocation.ty
                )));
            }
            let (name, parent) = names::definition(hir, interner, *def);
            let symbol = names::static_item(&name, parent.as_deref());
            let (global, image) = declare_one(
                module,
                values,
                &symbol,
                data.ty,
                &allocation.value,
                !data.mutable,
                Linkage::Internal,
            )?;
            globals.initializers.push((global, image));
            globals.order.push(*def);
            globals.statics.insert(
                *def,
                StaticGlobal {
                    global,
                    ty: data.ty,
                },
            );
            globals.allocations.insert(data.alloc, global);
        }

        for id in mono.const_allocations() {
            // A static's allocation is declared with the static and must not
            // receive a second global with a different address.
            if globals.allocations.contains_key(id) {
                continue;
            }
            let allocation = program.allocation(*id);
            let symbol = allocation_name(hir, interner, program, *id)?;
            let (global, image) = declare_one(
                module,
                values,
                &symbol,
                allocation.ty,
                &allocation.value,
                !allocation.mutable,
                Linkage::Private,
            )?;
            globals.initializers.push((global, image));
            globals.allocations.insert(*id, global);
        }

        Ok(globals)
    }

    /// Fills every declared global with its encoded initializer.
    pub(crate) fn initialize(
        &self,
        values: &ConstValues<'_, 'ctx>,
        pointers: &Pointers<'_, 'ctx>,
    ) -> Result<(), CodegenError> {
        for (global, image) in &self.initializers {
            values.set_initializer(*global, image, pointers)?;
        }
        Ok(())
    }

    /// Returns the address of the static `def`.
    pub(crate) fn static_pointer(&self, def: DefId) -> Result<PointerValue<'ctx>, CodegenError> {
        let storage = self.statics.get(&def).ok_or_else(|| {
            CodegenError::internal(format!("static {def:?} was not declared as a global"))
        })?;
        Ok(storage.global.as_pointer_value())
    }

    /// Returns the declared type of the static `def`.
    pub(crate) fn static_type(&self, def: DefId) -> Option<TyId> {
        self.statics.get(&def).map(|storage| storage.ty)
    }

    /// Returns every declared static, in declaration order.
    pub(crate) fn statics(&self) -> impl Iterator<Item = (DefId, GlobalValue<'ctx>, TyId)> + '_ {
        self.order.iter().filter_map(|def| {
            self.statics
                .get(def)
                .map(|storage| (*def, storage.global, storage.ty))
        })
    }

    /// Returns the global holding the allocation `id`.
    pub(crate) fn allocation(&self, id: AllocId) -> Result<GlobalValue<'ctx>, CodegenError> {
        self.allocations.get(&id).copied().ok_or_else(|| {
            CodegenError::internal(format!(
                "allocation {id:?} is referenced but was never declared as a global"
            ))
        })
    }
}

/// Declares one global of `ty` initialized with `value`.
fn declare_one<'ctx>(
    module: &Module<'ctx>,
    values: &ConstValues<'_, 'ctx>,
    symbol: &str,
    ty: TyId,
    value: &ConstValue,
    constant: bool,
    linkage: Linkage,
) -> Result<(GlobalValue<'ctx>, Image), CodegenError> {
    let image = values.encode(ty, value)?;
    let value_type = values.image_type(&image)?;
    let global = module.add_global(value_type, None, symbol);
    global.set_linkage(linkage);
    global.set_constant(constant);
    global.set_alignment(values.alignment(ty)?);
    Ok((global, image))
}

/// Returns the linker symbol of one const allocation.
fn allocation_name(
    hir: &Hir,
    interner: &Interner,
    program: &ConstProgram,
    id: AllocId,
) -> Result<String, CodegenError> {
    Ok(match program.allocation(id).source {
        AllocSource::Const(def) => {
            let (name, parent) = names::definition(hir, interner, def);
            names::constant(&name, parent.as_deref())
        }
        AllocSource::Promoted { owner, index } => {
            let (name, parent) = names::definition(hir, interner, owner);
            let owner = match parent {
                Some(parent) => format!("{parent}.{name}"),
                None => name,
            };
            names::promoted(&owner, index)
        }
        AllocSource::ConstFnLocal { instance, index } => {
            names::const_fn_local(seal_mono::instance_hash(hir, instance), index)
        }
        AllocSource::Static(def) => {
            return Err(CodegenError::internal(format!(
                "allocation {id:?} of static {def:?} was listed as a constant allocation"
            )));
        }
    })
}
