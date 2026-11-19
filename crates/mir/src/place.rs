//! Storage locations.

use seal_def::DefId;
use seal_ty::VariantIdx;

use crate::id::LocalId;

/// One step from a place to a sub-place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Projection {
    /// Reads through a shared reference.
    Deref,
    /// Reads through a raw pointer.
    ///
    /// The pointee is not tracked by initialization or drop analysis: its
    /// storage belongs to the unsafe contract, not to this frame.
    RawDeref,
    /// Selects a struct field by its definition.
    Field(DefId),
    /// Selects a tuple element by its zero-based position.
    TupleField(u32),
    /// Selects an array element by a runtime index.
    Index {
        /// The local holding the evaluated `usize` index.
        local: LocalId,
        /// The constant index when the source used an in-range literal.
        constant: Option<u64>,
    },
    /// Selects one field of one enum variant.
    ///
    /// The variant is part of the projection because the field lives inside the
    /// payload at an offset that depends on which variant is active. Reaching
    /// this projection is only valid after the tag proved the variant.
    VariantField {
        /// The variant whose payload is read.
        variant: VariantIdx,
        /// The field definition inside that variant.
        field: DefId,
    },
}

/// The storage a place starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlaceRoot {
    /// A local slot in the body, including the return place and parameters.
    Local(LocalId),
    /// A static item. Its allocation has program lifetime.
    Static(DefId),
    /// A constant item used as a place. It is always materialized into a
    /// temporary first, so a const root never survives into code generation.
    Const(DefId),
}

/// A storage location: a root plus the projections applied to it.
///
/// The place is a value with its own projection list. Assignments and borrows
/// address any prefix of it, which is what move analysis needs to compare two
/// places.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Place {
    /// The storage the place starts from.
    pub root: PlaceRoot,
    /// The projections in application order.
    pub projections: Vec<Projection>,
}

impl Place {
    /// Creates a place for a whole local.
    #[inline]
    pub const fn local(local: LocalId) -> Self {
        Self {
            root: PlaceRoot::Local(local),
            projections: Vec::new(),
        }
    }

    /// Creates a place for a whole static.
    #[inline]
    pub const fn static_root(def: DefId) -> Self {
        Self {
            root: PlaceRoot::Static(def),
            projections: Vec::new(),
        }
    }

    /// Creates a place for a whole constant, before it is materialized.
    #[inline]
    pub const fn const_root(def: DefId) -> Self {
        Self {
            root: PlaceRoot::Const(def),
            projections: Vec::new(),
        }
    }

    /// Creates an already-projected place rooted at a local.
    #[inline]
    pub fn with_projections(local: LocalId, projections: Vec<Projection>) -> Self {
        Self {
            root: PlaceRoot::Local(local),
            projections,
        }
    }

    /// Creates an already-projected place rooted at `root`.
    #[inline]
    pub fn with_root(root: PlaceRoot, projections: Vec<Projection>) -> Self {
        Self { root, projections }
    }

    /// Returns the local root, or `None` for a const or static root.
    #[inline]
    pub const fn local_id(&self) -> Option<LocalId> {
        match self.root {
            PlaceRoot::Local(local) => Some(local),
            PlaceRoot::Static(_) | PlaceRoot::Const(_) => None,
        }
    }

    /// Returns the local root.
    ///
    /// # Panics
    ///
    /// Panics when the place is rooted at a const or static. Phases that do not
    /// yet model global storage use [`Self::local_id`] to skip such places.
    #[inline]
    pub fn expect_local(&self) -> LocalId {
        self.local_id()
            .unwrap_or_else(|| panic!("place is rooted at {:?}, not a local", self.root))
    }

    /// Returns the place reached by one dereference.
    #[inline]
    pub fn deref(mut self) -> Self {
        self.projections.push(Projection::Deref);
        self
    }

    /// Returns the place reached by one raw-pointer dereference.
    #[inline]
    pub fn raw_deref(mut self) -> Self {
        self.projections.push(Projection::RawDeref);
        self
    }

    /// Returns the place reached by selecting a field.
    #[inline]
    pub fn field(mut self, field: DefId) -> Self {
        self.projections.push(Projection::Field(field));
        self
    }

    /// Returns the place reached by selecting tuple element `index`.
    #[inline]
    pub fn tuple_field(mut self, index: u32) -> Self {
        self.projections.push(Projection::TupleField(index));
        self
    }

    /// Returns the place reached by indexing an array.
    #[inline]
    pub fn index(mut self, local: LocalId, constant: Option<u64>) -> Self {
        self.projections.push(Projection::Index { local, constant });
        self
    }

    /// Returns the place reached by selecting a field of an enum variant.
    #[inline]
    pub fn variant_field(mut self, variant: VariantIdx, field: DefId) -> Self {
        self.projections
            .push(Projection::VariantField { variant, field });
        self
    }

    /// Returns `true` when the place selects an enum field through a variant.
    #[inline]
    pub fn has_variant_field(&self) -> bool {
        self.projections
            .iter()
            .any(|projection| matches!(projection, Projection::VariantField { .. }))
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
            .filter(|projection| matches!(projection, Projection::Deref | Projection::RawDeref))
            .count()
    }

    /// Returns `true` if the place is reached through at least one reference.
    ///
    /// Reading such a place copies, because a shared reference never gives up
    /// ownership of its pointee.
    #[inline]
    pub fn is_deref(&self) -> bool {
        self.projections
            .iter()
            .any(|projection| matches!(projection, Projection::Deref | Projection::RawDeref))
    }

    /// Returns `true` if the place is reached through a raw pointer.
    #[inline]
    pub fn is_raw_deref(&self) -> bool {
        self.projections.contains(&Projection::RawDeref)
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
        self.root == other.root
            && other.projections.len() >= self.projections.len()
            && self
                .projections
                .iter()
                .zip(&other.projections)
                .all(|(left, right)| left == right)
    }
}
