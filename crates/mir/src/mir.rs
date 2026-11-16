//! The MIR of one file.

use seal_def::DefId;
use seal_hash::FxHashMap;
use seal_index::IndexVec;

use crate::body::Body;
use crate::id::BodyId;
use crate::validate::{ValidationError, validate_body};

/// All lowered bodies of a file.
#[derive(Clone, Debug, Default)]
pub struct Mir {
    /// Bodies in HIR order.
    pub bodies: IndexVec<BodyId, Body>,
    by_owner: FxHashMap<DefId, BodyId>,
}

impl Mir {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push_body(&mut self, body: Body) -> BodyId {
        let owner = body.owner;
        let id = self.bodies.push(body);
        self.by_owner.insert(owner, id);
        id
    }

    /// Returns all bodies, indexable by [`BodyId`].
    #[inline]
    pub fn bodies(&self) -> &IndexVec<BodyId, Body> {
        &self.bodies
    }

    /// Returns a body.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this tree.
    #[inline]
    pub fn body(&self, id: BodyId) -> &Body {
        &self.bodies[id]
    }

    /// Returns the body owned by `owner`, if it has one.
    pub fn body_of(&self, owner: DefId) -> Option<&Body> {
        self.by_owner.get(&owner).map(|id| &self.bodies[*id])
    }

    /// Checks the structural invariants of every body.
    pub fn validate(&self) -> Result<(), ValidationError> {
        for body in self.bodies.iter() {
            validate_body(body)?;
        }
        Ok(())
    }
}
