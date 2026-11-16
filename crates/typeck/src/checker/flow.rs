//! Definite-initialization state.

use seal_def::DefId;
use seal_hash::FxHashSet;

/// Which locals are definitely initialized at one program point, and whether
/// that point is unreachable.
#[derive(Clone, Debug)]
pub(crate) struct Flow {
    initialized: FxHashSet<DefId>,
    pub(crate) diverges: bool,
}

impl Flow {
    pub(crate) fn new() -> Self {
        Self {
            initialized: FxHashSet::default(),
            diverges: false,
        }
    }

    pub(crate) fn is_initialized(&self, local: DefId) -> bool {
        self.initialized.contains(&local)
    }

    pub(crate) fn mark_initialized(&mut self, local: DefId) {
        self.initialized.insert(local);
    }

    pub(crate) fn mark_uninitialized(&mut self, local: DefId) {
        self.initialized.remove(&local);
    }

    /// Joins the states after the two arms of a conditional.
    ///
    /// A local is initialized afterwards only when both continuing paths
    /// initialize it. A path that diverged never reaches the join, so its state
    /// is ignored. When both paths diverge, the join is unreachable.
    pub(crate) fn merge(&mut self, then: Self, otherwise: Self) {
        if then.diverges && otherwise.diverges {
            self.diverges = true;
            self.initialized = then.initialized;
        } else if then.diverges {
            *self = otherwise;
        } else if otherwise.diverges {
            *self = then;
        } else {
            let mut initialized = FxHashSet::default();
            for local in &then.initialized {
                if otherwise.initialized.contains(local) {
                    initialized.insert(*local);
                }
            }
            self.initialized = initialized;
            self.diverges = false;
        }
    }
}
