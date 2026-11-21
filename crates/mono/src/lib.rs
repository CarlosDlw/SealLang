//! Monomorphization: from generic MIR to concrete instances.
//!
//! A *monomorphic instance* is one function together with one complete list of
//! type arguments. The collector starts at every non-generic function, walks
//! each body's calls, and records every reachable instance exactly once. A call
//! through a trait bound is resolved here: the concrete self type and trait
//! arguments select the unique implementation, and the call becomes a direct
//! call to that impl method's instance.
//!
//! Vtables are collected from the same specialized bodies. One [`MonoVTable`]
//! exists per distinct concrete pointee and principal trait; its entries are
//! the concrete method instances in [`seal_layout::VtableLayout`] order,
//! including supertrait methods that no body calls directly. Vtable IDs are
//! assigned in first-coercion order and stay independent from instance IDs.
//!
//! Drop glue is collected from the same bodies. Every drop site whose place
//! needs a destructor, and every trait object, requests one [`DropGlue`] per
//! concrete type. The shape names the destructor instance to call and the
//! children to drop afterwards, so code generation builds one function per
//! entry and points the first slot of a vtable at it.
//!
//! The result is a [`MonoProgram`]: dense [`InstanceId`]s and [`VtableId`]s in
//! deterministic discovery order, one [`MonoMir`] per instance with every type
//! substituted, and no parameter, inference variable, or symbolic call left.
//! Code generation and layout consume this program instead of the generic MIR.
//!
//! # Limits
//!
//! Expansion is bounded by [`MAX_DEPTH`] along any call chain, by
//! [`MAX_INSTANCES`] overall, by [`MAX_VTABLES`] for trait-object tables, by
//! [`MAX_GLUES`] for drop glue items, and by [`MAX_GLUE_DEPTH`] for one glue's
//! nesting. All report a diagnostic with the offending instance or coercion instead of
//! consuming unbounded memory; a growing instantiation such as `f<T>` calling
//! `f<[T; 1]>` therefore fails deterministically.

#![warn(missing_docs)]

mod collector;
mod drop;
mod error;
mod hash;
mod id;
mod substitute;
#[cfg(test)]
mod tests;
mod validate;
mod vtable;

use seal_const_eval::{AllocId, ConstProgram};
use seal_def::DefId;
use seal_hash::FxHashMap;
use seal_index::{Idx, IndexVec};
use seal_mir::Body;
pub use seal_ty::Instance;
use seal_ty::TyId;

pub use collector::{
    MAX_DEPTH, MAX_GLUE_DEPTH, MAX_GLUES, MAX_INSTANCES, MAX_VTABLES, MonoItem, collect,
    collect_with_specializer,
};
pub use drop::{DropGlue, DropGlueEntry, GlueChildren};
pub use error::MonoError;
pub use hash::{instance_hash, instance_name, type_hash, vtable_hash, vtable_name};
pub use id::{DropGlueId, InstanceId, VtableId};
pub use seal_mir::CallTarget;
pub use substitute::{BodyEvent, ChildCall, SpecializedBody, specialize};
pub use validate::MonoValidationError;
pub use vtable::{MonoVTable, VtableKey};

/// The MIR of one concrete instance.
///
/// Every type in `body` has the instance's type arguments substituted: locals,
/// aggregate arguments, literal types, and call targets. A symbolic trait call
/// has already been replaced by a direct call to the selected impl method.
#[derive(Clone, Debug, PartialEq)]
pub struct MonoMir {
    /// The definition and its complete type arguments.
    pub instance: Instance,
    /// The specialized body.
    pub body: Body,
}

/// Every reachable instance in deterministic discovery order.
#[derive(Clone, Debug, Default)]
pub struct MonoProgram {
    instances: IndexVec<InstanceId, MonoMir>,
    ids: FxHashMap<Instance, InstanceId>,
    items: Vec<MonoItem>,
    statics: Vec<DefId>,
    extern_statics: Vec<DefId>,
    const_allocations: Vec<AllocId>,
    vtables: IndexVec<VtableId, MonoVTable>,
    vtable_ids: FxHashMap<VtableKey, VtableId>,
    drop_glues: IndexVec<DropGlueId, DropGlueEntry>,
    drop_glue_ids: FxHashMap<TyId, DropGlueId>,
}

/// Everything one collection produced.
///
/// The collector builds this and hands it to [`MonoProgram::new`], so the
/// constructor keeps one argument as collection grows new item kinds.
pub(crate) struct Collected {
    pub(crate) instances: IndexVec<InstanceId, MonoMir>,
    pub(crate) ids: FxHashMap<Instance, InstanceId>,
    pub(crate) items: Vec<MonoItem>,
    pub(crate) statics: Vec<DefId>,
    pub(crate) extern_statics: Vec<DefId>,
    pub(crate) const_allocations: Vec<AllocId>,
    pub(crate) vtables: IndexVec<VtableId, MonoVTable>,
    pub(crate) vtable_ids: FxHashMap<VtableKey, VtableId>,
    pub(crate) drop_glues: IndexVec<DropGlueId, DropGlueEntry>,
    pub(crate) drop_glue_ids: FxHashMap<TyId, DropGlueId>,
}

impl MonoProgram {
    pub(crate) fn new(collected: Collected) -> Self {
        Self {
            instances: collected.instances,
            ids: collected.ids,
            items: collected.items,
            statics: collected.statics,
            extern_statics: collected.extern_statics,
            const_allocations: collected.const_allocations,
            vtables: collected.vtables,
            vtable_ids: collected.vtable_ids,
            drop_glues: collected.drop_glues,
            drop_glue_ids: collected.drop_glue_ids,
        }
    }

    /// Returns the instances in discovery order.
    #[inline]
    pub fn instances(&self) -> &IndexVec<InstanceId, MonoMir> {
        &self.instances
    }

    /// Returns one instance's MIR.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this program.
    #[inline]
    pub fn instance(&self, id: InstanceId) -> &MonoMir {
        &self.instances[id]
    }

    /// Returns every collected item in deterministic discovery order.
    #[inline]
    pub fn items(&self) -> &[MonoItem] {
        &self.items
    }

    /// Returns every reachable static, in discovery order.
    #[inline]
    pub fn statics(&self) -> &[DefId] {
        &self.statics
    }

    /// Returns every external static referenced by a collected body.
    ///
    /// External statics have no Seal allocation: the linker resolves the
    /// symbol, so they are recorded for validation but never collected as
    /// items.
    #[inline]
    pub fn extern_statics(&self) -> &[DefId] {
        &self.extern_statics
    }

    /// Returns every reachable non-static const allocation, in discovery order.
    #[inline]
    pub fn const_allocations(&self) -> &[AllocId] {
        &self.const_allocations
    }

    /// Returns the dense ID assigned to `instance`, if it was collected.
    #[inline]
    pub fn id_of(&self, instance: Instance) -> Option<InstanceId> {
        self.ids.get(&instance).copied()
    }

    /// Returns every collected vtable in first-coercion order.
    #[inline]
    pub fn vtables(&self) -> &IndexVec<VtableId, MonoVTable> {
        &self.vtables
    }

    /// Returns one vtable.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this program.
    #[inline]
    pub fn vtable(&self, id: VtableId) -> &MonoVTable {
        &self.vtables[id]
    }

    /// Returns the ID assigned to `key`, if its vtable was collected.
    #[inline]
    pub fn vtable_id(&self, key: VtableKey) -> Option<VtableId> {
        self.vtable_ids.get(&key).copied()
    }

    /// Returns every collected drop glue in first-request order.
    #[inline]
    pub fn drop_glues(&self) -> &IndexVec<DropGlueId, DropGlueEntry> {
        &self.drop_glues
    }

    /// Returns one glue entry.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this program.
    #[inline]
    pub fn drop_glue(&self, id: DropGlueId) -> &DropGlueEntry {
        &self.drop_glues[id]
    }

    /// Returns the ID assigned to the glue of `ty`, if it was collected.
    #[inline]
    pub fn drop_glue_id(&self, ty: TyId) -> Option<DropGlueId> {
        self.drop_glue_ids.get(&ty).copied()
    }

    /// Returns `true` when the program contains no instances.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    /// Checks every instance, vtable, and coercion invariant.
    pub fn validate(&self, hir: &seal_hir::Hir) -> Result<(), MonoValidationError> {
        for instance in &self.instances {
            instance.validate(hir, self)?;
        }
        for (index, table) in self.vtables.iter().enumerate() {
            let id = VtableId::from_usize(index);
            validate::validate_vtable(hir, self, id, table)?;
        }
        Ok(())
    }

    /// Validates the collected global items against the evaluated constants.
    pub fn validate_globals(&self, constants: &ConstProgram) -> Result<(), MonoValidationError> {
        validate::validate_globals(self, constants)
    }
}
