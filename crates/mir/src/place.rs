//! Storage locations.

use crate::id::LocalId;

/// A storage location: a local plus the dereferences applied to it.
///
/// The current language has no fields or indices, so the only projection is a
/// dereference: `*p` is the place rooted at the local holding `p` with one
/// projection. Aggregate projections are added here when aggregates arrive;
/// callers already receive a `Place` rather than a local ID, so adding them is
/// not a breaking change for consumers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Place {
    /// The local this place starts from.
    pub local: LocalId,
    /// The number of dereference projections applied to the local.
    pub derefs: u32,
}

impl Place {
    /// Creates a place for a whole local.
    #[inline]
    pub const fn local(local: LocalId) -> Self {
        Self { local, derefs: 0 }
    }

    /// Returns the place reached by one dereference.
    ///
    /// # Panics
    ///
    /// Panics if the projection count would overflow `u32`. Lowering bounds
    /// nesting far below this, so the panic is an internal invariant check.
    #[inline]
    pub fn deref(self) -> Self {
        Self {
            local: self.local,
            derefs: self.derefs + 1,
        }
    }

    /// Returns the number of dereference projections.
    #[inline]
    pub const fn derefs(self) -> u32 {
        self.derefs
    }

    /// Returns `true` if the place is reached through at least one reference.
    ///
    /// Reading such a place copies, because a shared reference never gives up
    /// ownership of its pointee.
    #[inline]
    pub const fn is_deref(self) -> bool {
        self.derefs > 0
    }
}
