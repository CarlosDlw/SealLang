//! The opt-in `Copy` registry.
//!
//! An aggregate is bitwise-copyable only when it explicitly declares
//! `impl Copy for T` and every field of every variant is `Copy`. The registry
//! records the declaration that opted in, keyed by [`AdtId`]: `Copy` is a
//! property of the type definition, not of one instantiation.
//!
//! Lowering fills the registry after validating every `impl Copy`, so a missing
//! entry always means "not opted in", never "not lowered yet". The query in
//! [`TyCtxt`](crate::TyCtxt) consults this registry before it inspects the
//! fields, and rechecks those fields with the actual arguments of each use.

use seal_def::DefId;
use seal_hash::FxHashMap;

use crate::id::AdtId;

/// The `Copy` declaration of each aggregate that opted in.
#[derive(Clone, Debug, Default)]
pub(crate) struct CopyImpls {
    impls: FxHashMap<AdtId, DefId>,
}

impl CopyImpls {
    /// Records `implementation` as the `Copy` declaration of `adt`.
    ///
    /// Returns the declaration previously recorded for the same aggregate, so
    /// the caller can report a duplicate implementation instead of silently
    /// replacing one.
    pub(crate) fn insert(&mut self, adt: AdtId, implementation: DefId) -> Option<DefId> {
        self.impls.insert(adt, implementation)
    }

    /// Returns the `Copy` declaration of `adt`, if the aggregate opted in.
    pub(crate) fn get(&self, adt: AdtId) -> Option<DefId> {
        self.impls.get(&adt).copied()
    }
}

#[cfg(test)]
mod tests {
    use seal_def::DefId;

    use super::CopyImpls;
    use crate::AdtId;

    fn def(index: u32) -> DefId {
        DefId::from_u32(index).expect("test definitions are small")
    }

    fn adt(index: u32) -> AdtId {
        AdtId::from_u32(index).expect("test aggregates are small")
    }

    #[test]
    fn inserting_a_declaration_returns_the_previous_one() {
        let mut impls = CopyImpls::default();

        assert_eq!(impls.get(adt(0)), None);
        assert_eq!(impls.insert(adt(0), def(10)), None);
        assert_eq!(impls.get(adt(0)), Some(def(10)));
        assert_eq!(impls.insert(adt(0), def(11)), Some(def(10)));
        assert_eq!(impls.get(adt(0)), Some(def(11)));
    }

    #[test]
    fn the_registry_keeps_aggregates_apart() {
        let mut impls = CopyImpls::default();

        assert_eq!(impls.insert(adt(0), def(10)), None);
        assert_eq!(impls.insert(adt(1), def(11)), None);
        assert_eq!(impls.get(adt(0)), Some(def(10)));
        assert_eq!(impls.get(adt(1)), Some(def(11)));
        assert_eq!(impls.get(adt(2)), None);
    }
}
