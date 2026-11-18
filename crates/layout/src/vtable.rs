//! Deterministic trait-object vtable layout.
//!
//! A vtable starts with three header entries followed by one pointer per
//! dispatchable method:
//!
//! ```text
//! slot 0: drop_in_place  fn(*mut u8)
//! slot 1: size           usize
//! slot 2: align          usize
//! slot 3..: method pointers
//! ```
//!
//! Methods are the principal trait's methods in declaration order, followed by
//! each supertrait depth-first in declaration order. A method reached through
//! several supertrait paths keeps only its first slot, keyed by the method's
//! declaration. Type arguments do not change the slot order, but they are
//! preserved in each entry's trait reference so monomorphization can select the
//! concrete implementation.

use std::collections::{BTreeMap, HashSet};
use std::fmt;

use seal_def::DefId;
use seal_ty::{TraitRef, TyCtxt, TyId};

/// Number of header entries before the first method slot.
pub const VTABLE_HEADER_SLOTS: u32 = 3;

/// The position of one entry in a vtable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VtableSlot(u32);

impl VtableSlot {
    /// The compiler-generated drop entry.
    pub const DROP: Self = Self(0);
    /// The size entry.
    pub const SIZE: Self = Self(1);
    /// The alignment entry.
    pub const ALIGN: Self = Self(2);

    /// Creates a slot from its zero-based index.
    pub const fn new(index: u32) -> Self {
        Self(index)
    }

    /// Returns the zero-based index of this slot.
    pub const fn index(self) -> u32 {
        self.0
    }
}

/// What one vtable entry contains.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VtableEntryKind {
    /// The reserved destructor slot; currently a compiler-generated no-op.
    Drop,
    /// The concrete value's size in bytes.
    Size,
    /// The concrete value's alignment in bytes.
    Align,
    /// A dispatchable method and the trait that declares it.
    Method {
        /// The method declaration, unique across the flattened trait graph.
        method: DefId,
        /// The instantiated supertrait reference that provides the method.
        declaring_trait: TraitRef,
    },
}

/// One ordered vtable entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VtableEntry {
    /// The entry's slot in the vtable.
    pub slot: VtableSlot,
    /// The entry's contents.
    pub kind: VtableEntryKind,
}

impl VtableEntry {
    /// Returns the method declaration when this is a method entry.
    pub const fn method(self) -> Option<DefId> {
        match self.kind {
            VtableEntryKind::Method { method, .. } => Some(method),
            VtableEntryKind::Drop | VtableEntryKind::Size | VtableEntryKind::Align => None,
        }
    }

    /// Returns the declaring trait reference when this is a method entry.
    pub const fn declaring_trait(self) -> Option<TraitRef> {
        match self.kind {
            VtableEntryKind::Method {
                declaring_trait, ..
            } => Some(declaring_trait),
            VtableEntryKind::Drop | VtableEntryKind::Size | VtableEntryKind::Align => None,
        }
    }
}

/// Why a vtable layout cannot be built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VtableLayoutError {
    /// The principal trait or one of its supertraits has no definition.
    MissingTrait(DefId),
    /// A trait reference supplies the wrong number of type arguments.
    MalformedTraitRef {
        /// The referenced trait.
        trait_def: DefId,
        /// The number of arguments the trait declares, excluding `Self`.
        expected: usize,
        /// The number of arguments the reference supplied.
        found: usize,
    },
    /// A supertrait reference could not be instantiated.
    MalformedSupertrait {
        /// The trait that declares the malformed supertrait.
        owner: DefId,
        /// The supertrait whose reference could not be instantiated.
        supertrait: DefId,
    },
    /// The supertrait graph reaches a trait that is already being visited.
    CyclicSupertrait(DefId),
    /// A trait-object layout was requested for a type that is not `dyn Trait`.
    NotTraitObject(TyId),
    /// The method count does not fit in a [`VtableSlot`].
    TooManyMethods,
}

impl fmt::Display for VtableLayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTrait(trait_def) => {
                write!(f, "trait {trait_def:?} has no definition")
            }
            Self::MalformedTraitRef {
                trait_def,
                expected,
                found,
            } => {
                write!(
                    f,
                    "trait {trait_def:?} expects {expected} type arguments, but received {found}"
                )
            }
            Self::MalformedSupertrait { owner, supertrait } => {
                write!(
                    f,
                    "trait {owner:?} declares a malformed supertrait reference to {supertrait:?}"
                )
            }
            Self::CyclicSupertrait(trait_def) => {
                write!(f, "trait {trait_def:?} is its own supertrait")
            }
            Self::NotTraitObject(ty) => write!(f, "type {ty:?} is not a trait object"),
            Self::TooManyMethods => write!(f, "the vtable has too many methods to address"),
        }
    }
}

impl std::error::Error for VtableLayoutError {}

/// The complete, deterministic slot assignment for one trait object.
///
/// The layout is a pure query over trait definitions and their instantiated
/// supertrait references. It never inspects the concrete implementing type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VtableLayout {
    principal: TraitRef,
    entries: Vec<VtableEntry>,
    slots: BTreeMap<DefId, VtableSlot>,
}

impl VtableLayout {
    /// Builds the vtable layout for `principal`.
    ///
    /// `principal.self_ty` is the concrete type in a monomorphized build and the
    /// `dyn` type when checking the object itself. It only participates in
    /// supertrait instantiation; slot assignment is independent of it.
    pub fn new(tcx: &TyCtxt, principal: TraitRef) -> Result<Self, VtableLayoutError> {
        let mut trait_refs = Vec::new();
        let mut visited = HashSet::new();
        let mut active = HashSet::new();
        collect_trait_refs(tcx, principal, &mut trait_refs, &mut visited, &mut active)?;

        let mut entries = vec![
            VtableEntry {
                slot: VtableSlot::DROP,
                kind: VtableEntryKind::Drop,
            },
            VtableEntry {
                slot: VtableSlot::SIZE,
                kind: VtableEntryKind::Size,
            },
            VtableEntry {
                slot: VtableSlot::ALIGN,
                kind: VtableEntryKind::Align,
            },
        ];
        let mut slots = BTreeMap::new();
        let mut seen = HashSet::new();

        for trait_ref in trait_refs {
            let definition = tcx
                .trait_def(trait_ref.trait_def)
                .ok_or(VtableLayoutError::MissingTrait(trait_ref.trait_def))?;
            for method in &definition.methods {
                if !seen.insert(method.def) {
                    continue;
                }
                let index =
                    u32::try_from(entries.len()).map_err(|_| VtableLayoutError::TooManyMethods)?;
                let slot = VtableSlot::new(index);
                entries.push(VtableEntry {
                    slot,
                    kind: VtableEntryKind::Method {
                        method: method.def,
                        declaring_trait: trait_ref,
                    },
                });
                slots.insert(method.def, slot);
            }
        }

        Ok(Self {
            principal,
            entries,
            slots,
        })
    }

    /// Builds the vtable layout for the trait object `object`.
    pub fn for_object(tcx: &TyCtxt, object: TyId) -> Result<Self, VtableLayoutError> {
        let (trait_def, args) = tcx
            .dyn_parts(object)
            .ok_or(VtableLayoutError::NotTraitObject(object))?;
        Self::new(tcx, TraitRef::new(trait_def, object, args))
    }

    /// Returns the principal trait reference this layout was built for.
    pub fn principal(&self) -> TraitRef {
        self.principal
    }

    /// Returns every entry in slot order, including the three header entries.
    pub fn entries(&self) -> &[VtableEntry] {
        &self.entries
    }

    /// Returns the method entries in slot order.
    pub fn method_entries(&self) -> impl Iterator<Item = &VtableEntry> {
        self.entries.iter().filter(|entry| entry.method().is_some())
    }

    /// Returns the number of method slots after the header.
    pub fn method_count(&self) -> usize {
        self.entries.len() - VTABLE_HEADER_SLOTS as usize
    }

    /// Returns the slot assigned to `method`, if the layout contains it.
    pub fn slot_of(&self, method: DefId) -> Option<VtableSlot> {
        self.slots.get(&method).copied()
    }

    /// Returns the entry at `slot`, if the slot exists.
    pub fn entry(&self, slot: VtableSlot) -> Option<VtableEntry> {
        self.entries
            .get(slot.index() as usize)
            .copied()
            .filter(|entry| entry.slot == slot)
    }
}

/// Flattens the principal and its supertraits depth-first in declaration order.
fn collect_trait_refs(
    tcx: &TyCtxt,
    current: TraitRef,
    refs: &mut Vec<TraitRef>,
    visited: &mut HashSet<TraitRef>,
    active: &mut HashSet<DefId>,
) -> Result<(), VtableLayoutError> {
    if !active.insert(current.trait_def) {
        return Err(VtableLayoutError::CyclicSupertrait(current.trait_def));
    }
    if !visited.insert(current) {
        active.remove(&current.trait_def);
        return Ok(());
    }
    let definition = tcx
        .trait_def(current.trait_def)
        .ok_or(VtableLayoutError::MissingTrait(current.trait_def))?;
    let expected = definition.generics.params.len().saturating_sub(1);
    let found = tcx.substs_len(current.args);
    if expected != found {
        return Err(VtableLayoutError::MalformedTraitRef {
            trait_def: current.trait_def,
            expected,
            found,
        });
    }
    refs.push(current);

    for supertrait in &definition.supertraits {
        let instantiated = tcx.instantiate_supertrait(*supertrait, current).ok_or(
            VtableLayoutError::MalformedSupertrait {
                owner: current.trait_def,
                supertrait: supertrait.trait_def,
            },
        )?;
        collect_trait_refs(tcx, instantiated, refs, visited, active)?;
    }
    active.remove(&current.trait_def);
    Ok(())
}

#[cfg(test)]
mod tests;
