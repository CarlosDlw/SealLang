//! Global effects of mutable statics.
//!
//! Borrow checking is intraprocedural, but a callee can access the same
//! `static mut` the caller borrowed. This analysis summarizes the statics each
//! body may read or write, unions direct callees and every possible trait or
//! dynamic dispatch target, and solves recursion with a monotone fixed point.
//!
//! A path is a static definition plus statically known fields and constant
//! indices. A runtime index collapses to the array path, so it covers every
//! element. A read is shared; a write is exclusive. Paths overlap when one is a
//! prefix of the other.

use std::collections::BTreeSet;

use crate::origins::{StorageField, StorageKey};
use seal_def::{DefId, DefKind};
use seal_hash::FxHashMap;
use seal_hir::Hir;
use seal_mir::{CallTarget, Mir, Place, PlaceRoot, Projection};

mod candidates;
mod collect;
mod conflicts;

use candidates::{dynamic_candidates, trait_candidates};
use collect::collect;
pub(crate) use conflicts::conflicts;

/// One step in a statically known path below a static.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum GlobalStep {
    /// A named struct or enum field.
    Field(DefId),
    /// A tuple element.
    TupleField(u32),
    /// A constant array index.
    Index(u64),
}

/// A static and a statically known path inside it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct GlobalPath {
    /// The static definition.
    pub(crate) def: DefId,
    /// The path below the static; empty for the whole allocation.
    pub(crate) steps: Vec<GlobalStep>,
}

impl GlobalPath {
    /// Returns whether one path contains the other.
    pub(crate) fn overlaps(&self, other: &Self) -> bool {
        self.def == other.def
            && (self.steps.starts_with(&other.steps) || other.steps.starts_with(&self.steps))
    }

    fn from_place(hir: &Hir, place: &Place) -> Option<Self> {
        let PlaceRoot::Static(def) = place.root else {
            return None;
        };
        if hir.definitions().kind(def) != DefKind::Static || !hir.definitions().is_mutable(def) {
            return None;
        }
        let mut steps = Vec::new();
        for projection in &place.projections {
            match projection {
                Projection::Field(field) | Projection::VariantField { field, .. } => {
                    steps.push(GlobalStep::Field(*field));
                }
                Projection::TupleField(index) => steps.push(GlobalStep::TupleField(*index)),
                Projection::Index {
                    constant: Some(index),
                    ..
                } => steps.push(GlobalStep::Index(*index)),
                // A runtime index names every element, and a dereference names
                // whatever the pointer targets: stop at the statically known
                // prefix and treat it as the whole remaining storage.
                Projection::Index { constant: None, .. } | Projection::Deref => break,
            }
        }
        Some(Self { def, steps })
    }

    fn from_storage(key: &StorageKey) -> Option<Self> {
        let StorageKey::Static { def, fields } = key else {
            return None;
        };
        let steps = fields
            .iter()
            .map(|field| match field {
                StorageField::Field(field) => GlobalStep::Field(*field),
                StorageField::TupleField(index) => GlobalStep::TupleField(*index),
            })
            .collect();
        Some(Self { def: *def, steps })
    }
}

/// The mutable statics one body may access.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct GlobalEffects {
    /// Shared accesses.
    pub(crate) reads: BTreeSet<GlobalPath>,
    /// Exclusive accesses.
    pub(crate) writes: BTreeSet<GlobalPath>,
    /// The body may call code with no visible body, so any static may be
    /// touched. This is defensive: safe Seal has no bodyless function.
    pub(crate) unknown: bool,
}

impl GlobalEffects {
    fn include(&mut self, other: &Self) -> bool {
        let mut changed = false;
        for path in &other.reads {
            changed |= self.reads.insert(path.clone());
        }
        for path in &other.writes {
            changed |= self.writes.insert(path.clone());
        }
        if other.unknown && !self.unknown {
            self.unknown = true;
            changed = true;
        }
        changed
    }

    fn read(&mut self, path: GlobalPath) {
        self.reads.insert(path);
    }

    fn write(&mut self, path: GlobalPath) {
        self.writes.insert(path);
    }
}

/// One call site's possible callees, with the effect information the checker
/// needs after the summaries have been solved.
#[derive(Default)]
pub(crate) struct Effects {
    summaries: FxHashMap<DefId, GlobalEffects>,
    trait_candidates: FxHashMap<DefId, Vec<DefId>>,
    dynamic_candidates: FxHashMap<DefId, Vec<DefId>>,
}

impl Effects {
    /// Returns the transitive effects of a direct or trait-bound call.
    pub(crate) fn call(&self, target: CallTarget) -> GlobalEffects {
        match target {
            CallTarget::Direct { instance, .. } => self
                .summaries
                .get(&instance.def)
                .cloned()
                .unwrap_or_else(GlobalEffects::unknown),
            CallTarget::TraitMethod { instance, .. } => {
                self.candidates(&self.trait_candidates, instance.def)
            }
        }
    }

    /// Returns the transitive effects of a dynamic call.
    pub(crate) fn dynamic(&self, method: DefId) -> GlobalEffects {
        self.candidates(&self.dynamic_candidates, method)
    }

    fn candidates(&self, table: &FxHashMap<DefId, Vec<DefId>>, method: DefId) -> GlobalEffects {
        let candidates = table.get(&method);
        let mut effects = GlobalEffects::default();
        match candidates {
            Some(candidates) if !candidates.is_empty() => {
                for callee in candidates {
                    if let Some(summary) = self.summaries.get(callee) {
                        effects.include(summary);
                    } else {
                        effects.unknown = true;
                    }
                }
            }
            _ => effects.unknown = true,
        }
        effects
    }
}

impl GlobalEffects {
    fn unknown() -> Self {
        Self {
            reads: BTreeSet::new(),
            writes: BTreeSet::new(),
            unknown: true,
        }
    }
}

/// One body's direct effects and the candidate callees of each call.
struct BodyEffects {
    owner: DefId,
    direct: GlobalEffects,
    calls: Vec<Vec<DefId>>,
}

/// Computes the effect summaries of every body in `mir`.
pub(crate) fn analyze(hir: &Hir, mir: &Mir) -> Effects {
    let trait_candidates = trait_candidates(hir);
    let dynamic_candidates = dynamic_candidates(hir);
    let bodies: Vec<BodyEffects> = mir
        .bodies()
        .iter()
        .map(|body| collect(hir, body, &trait_candidates, &dynamic_candidates))
        .collect();
    let mut summaries: FxHashMap<DefId, GlobalEffects> = bodies
        .iter()
        .map(|body| (body.owner, GlobalEffects::default()))
        .collect();
    loop {
        let mut changed = false;
        for body in &bodies {
            let mut summary = body.direct.clone();
            for candidates in &body.calls {
                if candidates.is_empty() {
                    summary.unknown = true;
                    continue;
                }
                for callee in candidates {
                    if let Some(effects) = summaries.get(callee) {
                        summary.include(effects);
                    } else {
                        summary.unknown = true;
                    }
                }
            }
            if let Some(existing) = summaries.get_mut(&body.owner) {
                changed |= existing.include(&summary);
            }
        }
        if !changed {
            break;
        }
    }
    Effects {
        summaries,
        trait_candidates,
        dynamic_candidates,
    }
}
