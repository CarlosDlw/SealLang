//! Definite initialization over move paths.
//!
//! A move path is a storage root plus the fields traversed from it. Type
//! checking and borrow checking both need the same lattice, so it lives here,
//! generic over the root handle: type checking uses a `DefId`, borrow checking
//! uses a MIR local.
//!
//! Fields are tracked with the nearest-entry rule: the state of a path is the
//! state of the longest recorded prefix. Assigning a path records
//! `Initialized`; moving it records `Moved`; recording a path drops descendant
//! entries because the parent now covers them. A whole read requires every
//! descendant field to be initialized and none to be moved, which is why the
//! lattice needs the field types.

use std::hash::Hash;

use seal_def::DefId;
use seal_hash::{FxHashMap, FxHashSet};

use crate::ctxt::TyCtxt;
use crate::id::{AdtId, TyId};
use crate::kind::TyKind;

/// One step of a move path inside a local.
///
/// Fields are selected by definition and array elements by their constant
/// index. A dynamic index cannot name one element at compile time, so callers
/// use the parent array path instead; the state of the whole array then covers
/// the access conservatively.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PathElem {
    /// A named struct field.
    Field(DefId),
    /// A constant array index.
    Index(u64),
}

/// The initialization state of one place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitState {
    /// No path from the root to this place is initialized.
    Uninitialized,
    /// Some, but not all, fields are initialized.
    PartiallyInitialized,
    /// The whole place is initialized.
    Initialized,
    /// The place is initialized, but one or more fields were moved out.
    PartiallyMoved,
    /// The place itself was moved out.
    Moved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    Initialized,
    Moved,
}

/// Definite-initialization facts for a set of move paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitPaths<K: Copy + Eq + Hash> {
    entries: FxHashMap<K, FxHashMap<Box<[PathElem]>, Entry>>,
}

impl<K: Copy + Eq + Hash> Default for InitPaths<K> {
    fn default() -> Self {
        Self {
            entries: FxHashMap::default(),
        }
    }
}

impl<K: Copy + Eq + Hash> InitPaths<K> {
    /// Creates an empty set of facts: every path is uninitialized.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that `root` plus `path` is initialized, covering descendants.
    pub fn assign(&mut self, root: K, path: &[PathElem]) {
        self.set(root, path, Entry::Initialized);
    }

    /// Records that `root` plus `path` was moved out.
    pub fn move_out(&mut self, root: K, path: &[PathElem]) {
        self.set(root, path, Entry::Moved);
    }

    fn set(&mut self, root: K, path: &[PathElem], entry: Entry) {
        let paths = self.entries.entry(root).or_default();
        paths.retain(|existing, _| !(existing.len() > path.len() && existing.starts_with(path)));
        paths.insert(path.into(), entry);
    }

    /// Returns the state of `root` plus `fields`.
    ///
    /// `root_ty` is the type of the place before any field is applied; it is
    /// needed to know which fields a whole read must cover.
    pub fn state(&self, root: K, path: &[PathElem], tcx: &TyCtxt, root_ty: TyId) -> InitState {
        match self.nearest(root, path) {
            Some(Entry::Moved) => InitState::Moved,
            Some(Entry::Initialized) => {
                if self.descendant_moved(root, path) {
                    InitState::PartiallyMoved
                } else {
                    InitState::Initialized
                }
            }
            None => {
                let ty = place_type(tcx, root_ty, path).unwrap_or_else(|| tcx.error_ty());
                if matches!(tcx.kind(ty), TyKind::Adt(_) | TyKind::Array { .. })
                    && self.fully_initialized(root, path, tcx, ty, &mut FxHashSet::default())
                {
                    InitState::Initialized
                } else if self.descendant_moved(root, path) {
                    InitState::PartiallyMoved
                } else if self.has_descendants(root, path) {
                    InitState::PartiallyInitialized
                } else {
                    InitState::Uninitialized
                }
            }
        }
    }

    /// Joins another fact set into this one at a control-flow merge.
    ///
    /// A path is definitely initialized only when both inputs initialize it. A
    /// path moved on either side is treated as moved, because the merged point
    /// may observe the moved value.
    pub fn join(&mut self, other: &Self) {
        let roots: FxHashSet<K> = self
            .entries
            .keys()
            .chain(other.entries.keys())
            .copied()
            .collect();

        for root in roots {
            let left = self.entries.get(&root).cloned();
            let right = other.entries.get(&root).cloned();
            let mut merged: FxHashMap<Box<[PathElem]>, Entry> = FxHashMap::default();

            match (left.as_ref(), right.as_ref()) {
                (Some(left), Some(right)) => {
                    let paths: FxHashSet<&Box<[PathElem]>> =
                        left.keys().chain(right.keys()).collect();
                    for path in paths {
                        let entry = match (left.get(path), right.get(path)) {
                            (Some(a), Some(b)) if a == b => *a,
                            (Some(_), Some(_)) => Entry::Moved,
                            (Some(Entry::Moved), None) | (None, Some(Entry::Moved)) => Entry::Moved,
                            // Initialized on one side only: not definite.
                            (Some(Entry::Initialized), None) | (None, Some(Entry::Initialized)) => {
                                continue;
                            }
                            (None, None) => continue,
                        };
                        merged.insert(path.clone(), entry);
                    }
                }
                (Some(left), None) => {
                    for (path, entry) in left {
                        if *entry == Entry::Moved {
                            merged.insert(path.clone(), Entry::Moved);
                        }
                    }
                }
                (None, Some(right)) => {
                    for (path, entry) in right {
                        if *entry == Entry::Moved {
                            merged.insert(path.clone(), Entry::Moved);
                        }
                    }
                }
                (None, None) => {}
            }

            if merged.is_empty() {
                self.entries.remove(&root);
            } else {
                self.entries.insert(root, merged);
            }
        }
    }

    fn nearest(&self, root: K, path: &[PathElem]) -> Option<Entry> {
        let paths = self.entries.get(&root)?;
        for len in (0..=path.len()).rev() {
            if let Some(entry) = paths.get(&path[..len]) {
                return Some(*entry);
            }
        }
        None
    }

    fn descendant_moved(&self, root: K, path: &[PathElem]) -> bool {
        self.entries.get(&root).is_some_and(|paths| {
            paths.iter().any(|(candidate, entry)| {
                *entry == Entry::Moved
                    && candidate.len() > path.len()
                    && candidate.starts_with(path)
            })
        })
    }

    fn has_descendants(&self, root: K, path: &[PathElem]) -> bool {
        self.entries.get(&root).is_some_and(|paths| {
            paths
                .keys()
                .any(|candidate| candidate.len() > path.len() && candidate.starts_with(path))
        })
    }

    fn fully_initialized(
        &self,
        root: K,
        path: &[PathElem],
        tcx: &TyCtxt,
        ty: TyId,
        visiting: &mut FxHashSet<AdtId>,
    ) -> bool {
        match self.nearest(root, path) {
            Some(Entry::Moved) => return false,
            Some(Entry::Initialized) => return !self.descendant_moved(root, path),
            None => {}
        }

        match tcx.kind(ty) {
            TyKind::Adt(adt) => {
                if !visiting.insert(adt) {
                    return false;
                }
                let def = tcx.adt(adt);
                if def.fields.is_empty() {
                    visiting.remove(&adt);
                    return false;
                }
                let initialized = def.fields.iter().all(|field| {
                    let mut nested = path.to_vec();
                    nested.push(PathElem::Field(field.def));
                    self.fully_initialized(root, &nested, tcx, field.ty, visiting)
                });
                visiting.remove(&adt);
                initialized
            }
            TyKind::Array { element, length } => {
                let recorded = self.entries.get(&root).map_or(0, |paths| paths.len());
                // Do not walk a huge length: a whole-array assignment was
                // already found by `nearest`, so per-index entries can only
                // cover as many elements as there are recorded paths.
                if length > recorded as u64 {
                    return false;
                }
                (0..length).all(|index| {
                    let mut nested = path.to_vec();
                    nested.push(PathElem::Index(index));
                    self.fully_initialized(root, &nested, tcx, element, visiting)
                })
            }
            _ => false,
        }
    }
}

/// Returns the type reached by applying `path` to `root_ty`.
pub fn place_type(tcx: &TyCtxt, root_ty: TyId, path: &[PathElem]) -> Option<TyId> {
    let mut ty = root_ty;
    for element in path {
        match (tcx.kind(ty), element) {
            (TyKind::Adt(adt), PathElem::Field(field)) => {
                ty = tcx.field(adt, *field)?.ty;
            }
            (TyKind::Array { element, length }, PathElem::Index(index)) => {
                if *index >= length {
                    return None;
                }
                ty = element;
            }
            _ => return None,
        }
    }
    Some(ty)
}

#[cfg(test)]
mod tests {
    use seal_def::DefId;
    use seal_symbol::Interner;

    use super::{InitPaths, InitState, PathElem};
    use crate::{AdtId, FieldDef, IntTy, TyCtxt, TyId};

    fn def(index: u32) -> DefId {
        DefId::from_u32(index).expect("test definitions are small")
    }

    fn point(ctxt: &TyCtxt, interner: &Interner) -> (TyId, AdtId) {
        let name = interner.intern("Point");
        let struct_def = def(0);
        let adt = ctxt.push_adt(struct_def, name);
        let int = ctxt.int_ty(IntTy::I32);
        ctxt.fill_adt(
            adt,
            vec![
                FieldDef {
                    def: def(1),
                    name: interner.intern("x"),
                    ty: int,
                },
                FieldDef {
                    def: def(2),
                    name: interner.intern("y"),
                    ty: int,
                },
            ],
        );
        (ctxt.adt_ty(adt), adt)
    }

    #[test]
    fn a_whole_assignment_initializes_every_field() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let (point, _) = point(&ctxt, &interner);
        let mut init: InitPaths<DefId> = InitPaths::new();
        init.assign(def(9), &[]);
        assert_eq!(
            init.state(def(9), &[], &ctxt, point),
            InitState::Initialized
        );
        assert_eq!(
            init.state(def(9), &[PathElem::Field(def(1))], &ctxt, point),
            InitState::Initialized
        );
    }

    #[test]
    fn fields_initialize_independently() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let (point, _) = point(&ctxt, &interner);
        let mut init: InitPaths<DefId> = InitPaths::new();
        init.assign(def(9), &[PathElem::Field(def(1))]);
        assert_eq!(
            init.state(def(9), &[PathElem::Field(def(1))], &ctxt, point),
            InitState::Initialized
        );
        assert_eq!(
            init.state(def(9), &[PathElem::Field(def(2))], &ctxt, point),
            InitState::Uninitialized
        );
        assert_eq!(
            init.state(def(9), &[], &ctxt, point),
            InitState::PartiallyInitialized
        );

        init.assign(def(9), &[PathElem::Field(def(2))]);
        assert_eq!(
            init.state(def(9), &[], &ctxt, point),
            InitState::Initialized
        );
    }

    #[test]
    fn moving_a_field_is_visible_from_the_whole_place() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let (point, _) = point(&ctxt, &interner);
        let mut init: InitPaths<DefId> = InitPaths::new();
        init.assign(def(9), &[]);
        init.move_out(def(9), &[PathElem::Field(def(1))]);
        assert_eq!(
            init.state(def(9), &[PathElem::Field(def(1))], &ctxt, point),
            InitState::Moved
        );
        assert_eq!(
            init.state(def(9), &[PathElem::Field(def(2))], &ctxt, point),
            InitState::Initialized
        );
        assert_eq!(
            init.state(def(9), &[], &ctxt, point),
            InitState::PartiallyMoved
        );
    }

    #[test]
    fn join_keeps_only_definite_facts() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let (point, _) = point(&ctxt, &interner);

        let mut left: InitPaths<DefId> = InitPaths::new();
        left.assign(def(9), &[]);
        let mut right: InitPaths<DefId> = InitPaths::new();
        left.join(&right);
        assert_eq!(
            left.state(def(9), &[], &ctxt, point),
            InitState::Uninitialized
        );

        right.assign(def(9), &[]);
        let mut merged: InitPaths<DefId> = InitPaths::new();
        merged.assign(def(9), &[]);
        merged.join(&right);
        assert_eq!(
            merged.state(def(9), &[], &ctxt, point),
            InitState::Initialized
        );

        let mut moved: InitPaths<DefId> = InitPaths::new();
        moved.move_out(def(9), &[]);
        merged.join(&moved);
        assert_eq!(merged.state(def(9), &[], &ctxt, point), InitState::Moved);
    }

    #[test]
    fn array_elements_initialize_independently() {
        let ctxt = TyCtxt::new();
        let int = ctxt.int_ty(IntTy::I32);
        let array = ctxt.array_ty(int, 2);
        let mut init: InitPaths<DefId> = InitPaths::new();

        init.assign(def(9), &[PathElem::Index(0)]);
        assert_eq!(
            init.state(def(9), &[PathElem::Index(0)], &ctxt, array),
            InitState::Initialized
        );
        assert_eq!(
            init.state(def(9), &[PathElem::Index(1)], &ctxt, array),
            InitState::Uninitialized
        );
        assert_eq!(
            init.state(def(9), &[], &ctxt, array),
            InitState::PartiallyInitialized
        );

        init.assign(def(9), &[PathElem::Index(1)]);
        assert_eq!(
            init.state(def(9), &[], &ctxt, array),
            InitState::Initialized
        );
    }

    #[test]
    fn moving_an_array_element_leaves_its_sibling_initialized() {
        let ctxt = TyCtxt::new();
        let int = ctxt.int_ty(IntTy::I32);
        let array = ctxt.array_ty(int, 2);
        let mut init: InitPaths<DefId> = InitPaths::new();
        init.assign(def(9), &[]);
        init.move_out(def(9), &[PathElem::Index(0)]);
        assert_eq!(
            init.state(def(9), &[PathElem::Index(0)], &ctxt, array),
            InitState::Moved
        );
        assert_eq!(
            init.state(def(9), &[PathElem::Index(1)], &ctxt, array),
            InitState::Initialized
        );
        assert_eq!(
            init.state(def(9), &[], &ctxt, array),
            InitState::PartiallyMoved
        );
    }

    #[test]
    fn array_paths_type_check_every_index() {
        let ctxt = TyCtxt::new();
        let int = ctxt.int_ty(IntTy::I32);
        let array = ctxt.array_ty(int, 2);
        assert_eq!(
            super::place_type(&ctxt, array, &[PathElem::Index(1)]),
            Some(int)
        );
        assert_eq!(super::place_type(&ctxt, array, &[PathElem::Index(2)]), None);
    }
}
