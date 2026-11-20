//! Definite-initialization state over move paths.

use seal_def::DefId;
use seal_ty::{InitPaths, InitState, PathElem, TyCtxt, TyId};

/// Which move paths are definitely initialized at one program point, and
/// whether that point is unreachable.
#[derive(Clone, Debug)]
pub(crate) struct Flow {
    init: InitPaths<DefId>,
    pub(crate) diverges: bool,
}

impl Flow {
    pub(crate) fn new() -> Self {
        Self {
            init: InitPaths::new(),
            diverges: false,
        }
    }

    /// Returns the initialization state of a place.
    pub(crate) fn state(
        &self,
        local: DefId,
        path: &[PathElem],
        root_ty: TyId,
        tcx: &TyCtxt,
    ) -> InitState {
        self.init.state(local, path, tcx, root_ty)
    }

    /// Records that a place was assigned, covering its descendants.
    pub(crate) fn assign(&mut self, local: DefId, path: &[PathElem]) {
        self.init.assign(local, path);
    }

    /// Records that a place was moved out, so later uses are rejected.
    pub(crate) fn move_out(&mut self, local: DefId, path: &[PathElem]) {
        self.init.move_out(local, path);
    }

    /// Joins the states after the two arms of a conditional.
    ///
    /// A place is initialized afterwards only when both continuing paths
    /// initialize it. A path that diverged never reaches the join, so its state
    /// is ignored. When both paths diverge, the join is unreachable.
    pub(crate) fn merge(&mut self, then: Self, otherwise: Self) {
        if then.diverges && otherwise.diverges {
            self.diverges = true;
            self.init = then.init;
        } else if then.diverges {
            *self = otherwise;
        } else if otherwise.diverges {
            *self = then;
        } else {
            let mut init = then.init;
            init.join(&otherwise.init);
            self.init = init;
            self.diverges = false;
        }
    }
}
