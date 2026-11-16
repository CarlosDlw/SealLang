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
    entries: FxHashMap<K, FxHashMap<Box<[DefId]>, Entry>>,
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

    /// Records that `root` plus `fields` is initialized, covering descendants.
    pub fn assign(&mut self, root: K, fields: &[DefId]) {
        self.set(root, fields, Entry::Initialized);
    }

    /// Records that `root` plus `fields` was moved out.
    pub fn move_out(&mut self, root: K, fields: &[DefId]) {
        self.set(root, fields, Entry::Moved);
    }

    fn set(&mut self, root: K, fields: &[DefId], entry: Entry) {
        let paths = self.entries.entry(root).or_default();
        paths.retain(|path, _| !(path.len() > fields.len() && path.starts_with(fields)));
        paths.insert(fields.into(), entry);
    }

    /// Returns the state of `root` plus `fields`.
    ///
    /// `root_ty` is the type of the place before any field is applied; it is
    /// needed to know which fields a whole read must cover.
    pub fn state(&self, root: K, fields: &[DefId], tcx: &TyCtxt, root_ty: TyId) -> InitState {
        match self.nearest(root, fields) {
            Some(Entry::Moved) => InitState::Moved,
            Some(Entry::Initialized) => {
                if self.descendant_moved(root, fields) {
                    InitState::PartiallyMoved
                } else {
                    InitState::Initialized
                }
            }
            None => {
                let ty = place_type(tcx, root_ty, fields).unwrap_or_else(|| tcx.error_ty());
                if matches!(tcx.kind(ty), TyKind::Adt(_))
                    && self.fully_initialized(root, fields, tcx, ty, &mut FxHashSet::default())
                {
                    InitState::Initialized
                } else if self.descendant_moved(root, fields) {
                    InitState::PartiallyMoved
                } else if self.has_descendants(root, fields) {
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
            let mut merged: FxHashMap<Box<[DefId]>, Entry> = FxHashMap::default();

            match (left.as_ref(), right.as_ref()) {
                (Some(left), Some(right)) => {
                    let paths: FxHashSet<&Box<[DefId]>> = left.keys().chain(right.keys()).collect();
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

    fn nearest(&self, root: K, fields: &[DefId]) -> Option<Entry> {
        let paths = self.entries.get(&root)?;
        for len in (0..=fields.len()).rev() {
            if let Some(entry) = paths.get(&fields[..len]) {
                return Some(*entry);
            }
        }
        None
    }

    fn descendant_moved(&self, root: K, fields: &[DefId]) -> bool {
        self.entries.get(&root).is_some_and(|paths| {
            paths.iter().any(|(path, entry)| {
                *entry == Entry::Moved && path.len() > fields.len() && path.starts_with(fields)
            })
        })
    }

    fn has_descendants(&self, root: K, fields: &[DefId]) -> bool {
        self.entries.get(&root).is_some_and(|paths| {
            paths
                .keys()
                .any(|path| path.len() > fields.len() && path.starts_with(fields))
        })
    }

    fn fully_initialized(
        &self,
        root: K,
        fields: &[DefId],
        tcx: &TyCtxt,
        ty: TyId,
        visiting: &mut FxHashSet<AdtId>,
    ) -> bool {
        match self.nearest(root, fields) {
            Some(Entry::Moved) => return false,
            Some(Entry::Initialized) => return !self.descendant_moved(root, fields),
            None => {}
        }

        let TyKind::Adt(adt) = tcx.kind(ty) else {
            return false;
        };
        if !visiting.insert(adt) {
            return false;
        }
        let def = tcx.adt(adt);
        if def.fields.is_empty() {
            visiting.remove(&adt);
            return false;
        }
        let initialized = def.fields.iter().all(|field| {
            let mut nested = fields.to_vec();
            nested.push(field.def);
            self.fully_initialized(root, &nested, tcx, field.ty, visiting)
        });
        visiting.remove(&adt);
        initialized
    }
}

/// Returns the type reached by applying `fields` to `root_ty`.
pub fn place_type(tcx: &TyCtxt, root_ty: TyId, fields: &[DefId]) -> Option<TyId> {
    let mut ty = root_ty;
    for field in fields {
        let TyKind::Adt(adt) = tcx.kind(ty) else {
            return None;
        };
        ty = tcx.field(adt, *field)?.ty;
    }
    Some(ty)
}

#[cfg(test)]
mod tests {
    use seal_def::DefId;
    use seal_symbol::Interner;

    use super::{InitPaths, InitState};
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
            init.state(def(9), &[def(1)], &ctxt, point),
            InitState::Initialized
        );
    }

    #[test]
    fn fields_initialize_independently() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let (point, _) = point(&ctxt, &interner);
        let mut init: InitPaths<DefId> = InitPaths::new();
        init.assign(def(9), &[def(1)]);
        assert_eq!(
            init.state(def(9), &[def(1)], &ctxt, point),
            InitState::Initialized
        );
        assert_eq!(
            init.state(def(9), &[def(2)], &ctxt, point),
            InitState::Uninitialized
        );
        assert_eq!(
            init.state(def(9), &[], &ctxt, point),
            InitState::PartiallyInitialized
        );

        init.assign(def(9), &[def(2)]);
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
        init.move_out(def(9), &[def(1)]);
        assert_eq!(
            init.state(def(9), &[def(1)], &ctxt, point),
            InitState::Moved
        );
        assert_eq!(
            init.state(def(9), &[def(2)], &ctxt, point),
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
}
