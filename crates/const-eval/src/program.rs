//! The evaluated constants of one file.
//!
//! Later phases never evaluate source again: they read this program. A missing
//! value is an internal invariant violation, not a user error.

use std::collections::BTreeMap;

use seal_def::DefId;
use seal_hash::FxHashMap;
use seal_ty::TyId;

use crate::value::{
    AllocId, Allocations, ConstAllocation, ConstValue, Dependency, StaticData, TraitObjectKey,
};

/// Every evaluated constant, static, and allocation of one file.
#[derive(Clone, Debug, Default)]
pub struct ConstProgram {
    pub(crate) values: FxHashMap<DefId, ConstValue>,
    pub(crate) statics: Vec<StaticData>,
    pub(crate) static_by_def: FxHashMap<DefId, usize>,
    pub(crate) allocations: Allocations,
    pub(crate) const_alloc: FxHashMap<DefId, AllocId>,
    pub(crate) dependencies: FxHashMap<DefId, Vec<Dependency>>,
    pub(crate) vtables: Vec<TraitObjectKey>,
}

impl ConstProgram {
    /// Returns the evaluated value of a const or static.
    pub fn value(&self, def: DefId) -> Option<&ConstValue> {
        self.values.get(&def)
    }

    /// Returns the program-lifetime data of a static.
    pub fn static_data(&self, def: DefId) -> Option<&StaticData> {
        self.static_by_def
            .get(&def)
            .map(|index| &self.statics[*index])
    }

    /// Returns every evaluated static, in definition order.
    pub fn statics(&self) -> &[StaticData] {
        &self.statics
    }

    /// Returns one allocation.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this program.
    pub fn allocation(&self, id: AllocId) -> &ConstAllocation {
        &self.allocations[id]
    }

    /// Returns the private allocation of an addressed const.
    pub fn const_allocation(&self, def: DefId) -> Option<AllocId> {
        self.const_alloc.get(&def).copied()
    }

    /// Returns every allocation.
    pub fn allocations(&self) -> &Allocations {
        &self.allocations
    }

    /// Returns the dependencies discovered for `def`, in discovery order.
    pub fn dependencies(&self, def: DefId) -> &[Dependency] {
        self.dependencies
            .get(&def)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Returns every trait-object key that needs a vtable.
    pub fn static_vtables(&self) -> &[TraitObjectKey] {
        &self.vtables
    }

    /// Returns the declared type of `def`, when it has an allocation.
    pub fn allocation_type(&self, def: DefId) -> Option<TyId> {
        self.static_data(def).map(|data| data.ty)
    }

    /// Returns every definition whose value is known, sorted by ID.
    ///
    /// Sorting by ID keeps diagnostics and debug output independent of hash
    /// iteration order.
    pub fn evaluated_definitions(&self) -> BTreeMap<DefId, &ConstValue> {
        self.values
            .iter()
            .map(|(def, value)| (*def, value))
            .collect()
    }
}
