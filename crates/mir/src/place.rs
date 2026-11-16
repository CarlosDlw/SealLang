//! Storage locations.

use seal_def::DefId;

use crate::id::LocalId;

/// One step from a place to a sub-place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Projection {
    /// Reads through a shared reference.
    Deref,
    /// Selects a struct field by its definition.
    Field(DefId),
}

/// A storage location: a local plus the projections applied to it.
///
/// The place is a value with its own projection list. Assignments and borrows
/// address any prefix of it, which is what move analysis needs to compare two
/// places.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Place {
    /// The local this place starts from.
    pub local: LocalId,
    /// The projections in application order.
    pub projections: Vec<Projection>,
}

impl Place {
    /// Creates a place for a whole local.
    #[inline]
    pub const fn local(local: LocalId) -> Self {
        Self {
            local,
            projections: Vec::new(),
        }
    }

    /// Creates an already-projected place.
    #[inline]
    pub fn with_projections(local: LocalId, projections: Vec<Projection>) -> Self {
        Self { local, projections }
    }

    /// Returns the place reached by one dereference.
    #[inline]
    pub fn deref(mut self) -> Self {
        self.projections.push(Projection::Deref);
        self
    }

    /// Returns the place reached by selecting a field.
    #[inline]
    pub fn field(mut self, field: DefId) -> Self {
        self.projections.push(Projection::Field(field));
        self
    }

    /// Returns the projections in application order.
    #[inline]
    pub fn projections(&self) -> &[Projection] {
        &self.projections
    }

    /// Returns the number of dereference projections.
    #[inline]
    pub fn derefs(&self) -> usize {
        self.projections
            .iter()
            .filter(|projection| **projection == Projection::Deref)
            .count()
    }

    /// Returns `true` if the place is reached through at least one reference.
    ///
    /// Reading such a place copies, because a shared reference never gives up
    /// ownership of its pointee.
    #[inline]
    pub fn is_deref(&self) -> bool {
        self.projections.contains(&Projection::Deref)
    }

    /// Returns `true` if the place names a whole local.
    #[inline]
    pub fn is_whole(&self) -> bool {
        self.projections.is_empty()
    }

    /// Returns `true` if `self` is a prefix of `other`.
    ///
    /// Move and initialization analysis compare places by path: a write to a
    /// prefix initializes every descendant, and a move out of a prefix
    /// invalidates every descendant.
    pub fn is_prefix_of(&self, other: &Place) -> bool {
        self.local == other.local
            && other.projections.len() >= self.projections.len()
            && self
                .projections
                .iter()
                .zip(&other.projections)
                .all(|(left, right)| left == right)
    }
}
