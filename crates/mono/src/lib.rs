//! Monomorphization: from generic MIR to concrete instances.
//!
//! A *monomorphic instance* is one function together with one complete list of
//! type arguments. The collector starts at every non-generic function, walks
//! each body's calls, and records every reachable instance exactly once. A call
//! through a trait bound is resolved here: the concrete self type and trait
//! arguments select the unique implementation, and the call becomes a direct
//! call to that impl method's instance.
//!
//! The result is a [`MonoProgram`]: dense [`InstanceId`]s in deterministic
//! discovery order, one [`MonoMir`] per instance with every type substituted,
//! and no parameter, inference variable, or symbolic call left. Code generation
//! and layout consume this program instead of the generic MIR.
//!
//! # Limits
//!
//! Expansion is bounded by [`MAX_DEPTH`] along any call chain and by
//! [`MAX_INSTANCES`] overall. Both report a diagnostic with the offending
//! instance instead of consuming unbounded memory; a growing instantiation such
//! as `f<T>` calling `f<[T; 1]>` therefore fails deterministically.

#![warn(missing_docs)]

mod collector;
mod error;
mod hash;
mod id;
mod substitute;
#[cfg(test)]
mod tests;
mod validate;

use seal_hash::FxHashMap;
use seal_index::IndexVec;
use seal_mir::Body;
pub use seal_ty::Instance;

pub use collector::{MAX_DEPTH, MAX_INSTANCES, MonoItem, collect};
pub use error::MonoError;
pub use hash::{instance_hash, instance_name};
pub use id::InstanceId;
pub use seal_mir::CallTarget;
pub use validate::MonoValidationError;

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
}

impl MonoProgram {
    pub(crate) fn new(
        instances: IndexVec<InstanceId, MonoMir>,
        ids: FxHashMap<Instance, InstanceId>,
        items: Vec<MonoItem>,
    ) -> Self {
        Self {
            instances,
            ids,
            items,
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

    /// Returns `true` when the program contains no instances.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    /// Checks every instance's type and call-target invariants.
    pub fn validate(&self, hir: &seal_hir::Hir) -> Result<(), MonoValidationError> {
        for instance in &self.instances {
            instance.validate(hir, self)?;
        }
        Ok(())
    }
}
