//! Storage locations.

use crate::id::LocalId;

/// A storage location.
///
/// The current language has no fields, indices, or dereference syntax, so a
/// place is always a whole local. Projections are added as fields here when
/// aggregates and borrows arrive; callers already receive a `Place` rather than
/// a local ID, so adding them is not a breaking change for consumers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Place {
    /// The local this place starts from.
    pub local: LocalId,
}

impl Place {
    /// Creates a place for a whole local.
    #[inline]
    pub const fn local(local: LocalId) -> Self {
        Self { local }
    }
}
