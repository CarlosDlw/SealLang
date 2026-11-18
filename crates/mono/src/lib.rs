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
//! The result is a [`MonoProgram`]: dense [`InstanceId`]s and [`VtableId`]s in
//! deterministic discovery order, one [`MonoMir`] per instance with every type
//! substituted, and no parameter, inference variable, or symbolic call left.
//! Code generation and layout consume this program instead of the generic MIR.
//!
//! # Limits
//!
//! Expansion is bounded by [`MAX_DEPTH`] along any call chain, by
//! [`MAX_INSTANCES`] overall, and by [`MAX_VTABLES`] for trait-object tables.
//! All report a diagnostic with the offending instance or coercion instead of
//! consuming unbounded memory; a growing instantiation such as `f<T>` calling
//! `f<[T; 1]>` therefore fails deterministically.

#![warn(missing_docs)]

mod collector;
mod error;
mod hash;
mod id;
mod substitute;
#[cfg(test)]
mod tests;
mod validate;
mod vtable;

use seal_hash::FxHashMap;
use seal_index::{Idx, IndexVec};
use seal_mir::Body;
pub use seal_ty::Instance;

pub use collector::{MAX_DEPTH, MAX_INSTANCES, MAX_VTABLES, MonoItem, collect};
pub use error::MonoError;
pub use hash::{instance_hash, instance_name, vtable_hash, vtable_name};
pub use id::{InstanceId, VtableId};
pub use seal_mir::CallTarget;
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
    vtables: IndexVec<VtableId, MonoVTable>,
    vtable_ids: FxHashMap<VtableKey, VtableId>,
}

impl MonoProgram {
    pub(crate) fn new(
        instances: IndexVec<InstanceId, MonoMir>,
        ids: FxHashMap<Instance, InstanceId>,
        items: Vec<MonoItem>,
        vtables: IndexVec<VtableId, MonoVTable>,
        vtable_ids: FxHashMap<VtableKey, VtableId>,
    ) -> Self {
        Self {
            instances,
            ids,
            items,
            vtables,
            vtable_ids,
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

    /// Returns every collected item, in the same order as the instances.
    #[inline]
    pub fn items(&self) -> &[MonoItem] {
        &self.items
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
}
