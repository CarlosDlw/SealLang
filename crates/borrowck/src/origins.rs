//! May-alias tracking for reference values.
//!
//! A reference chooses the storage its pointee names, so two places reached
//! through references denote the same storage when the references may name the
//! same storage. This analysis computes, for every reference path in one frame,
//! the storage its pointee may name.
//!
//! The domain mirrors the may-hold analysis: a flow-sensitive map from local
//! paths to sets, joined at control-flow merges. The value differs: a set holds
//! [`StorageKey`]s, and the transfer of `&place` resolves the borrowed place
//! against the current state.

use std::collections::VecDeque;

use seal_def::DefId;
use seal_hash::FxHashSet;
use seal_hir::Hir;
use seal_mir::{
    Body, LocalId, LocalKind, Operand, Place, Projection, Rvalue, Statement, StatementKind,
};
use seal_ty::{TyId, TyKind};

/// A storage location a reference may point at.
///
/// A key either names storage this frame owns (`Frame`) or the pointee of a
/// reference whose target lies outside the frame (`Pointee`). The two never
/// overlap: a local's own storage and the data its reference points at are
/// different allocations.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum StorageKey {
    /// Storage of a local path in this frame.
    Frame {
        /// The local that owns the storage.
        local: LocalId,
        /// The field path below the local; empty for the whole local.
        fields: Vec<DefId>,
    },
    /// Storage reachable through a reference whose target is outside this frame.
    Pointee {
        /// The reference local that names the storage.
        local: LocalId,
        /// The field path below the pointee.
        fields: Vec<DefId>,
    },
}

/// The storage a reference value may name.
///
/// `unknown` makes the value alias every live loan: it is set when the
/// reference itself was loaded from memory, because the checker does not model
/// the contents of borrowed storage.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OriginSet {
    keys: FxHashSet<StorageKey>,
    unknown: bool,
}

impl OriginSet {
    /// Returns a set naming exactly the pointee of `local`, a reference whose
    /// target is outside this frame.
    fn pointee(local: LocalId) -> Self {
        let mut set = Self::default();
        set.insert(StorageKey::Pointee {
            local,
            fields: Vec::new(),
        });
        set
    }

    /// Returns a set that may name any storage.
    pub(crate) fn unknown() -> Self {
        Self {
            keys: FxHashSet::default(),
            unknown: true,
        }
    }

    fn frame(local: LocalId, fields: Vec<DefId>) -> Self {
        let mut set = Self::default();
        set.insert(StorageKey::Frame { local, fields });
        set
    }

    /// Returns `true` when the set names no storage at all.
    pub(crate) fn is_empty(&self) -> bool {
        self.keys.is_empty() && !self.unknown
    }

    /// Returns `true` when the set may name any storage.
    pub(crate) fn is_unknown(&self) -> bool {
        self.unknown
    }

    fn insert(&mut self, key: StorageKey) {
        self.keys.insert(key);
    }

    fn insert_unknown(&mut self) {
        self.unknown = true;
    }

    /// Unions `other` into this set, returning whether anything changed.
    fn extend(&mut self, other: &Self) -> bool {
        let mut changed = false;
        if other.unknown && !self.unknown {
            self.unknown = true;
            changed = true;
        }
        for key in &other.keys {
            changed |= self.keys.insert(key.clone());
        }
        changed
    }
}

/// A local's reference paths and the storage they may name.
#[derive(Clone, Debug, Default)]
struct LocalOrigins {
    paths: Vec<(Vec<Projection>, OriginSet)>,
}

/// The storage every reference path may name at one program point.
#[derive(Clone, Debug)]
pub(crate) struct OriginState {
    locals: Vec<LocalOrigins>,
}

impl OriginState {
    fn new(local_count: usize) -> Self {
        Self {
            locals: vec![LocalOrigins::default(); local_count],
        }
    }

    /// Returns the storage the reference at `place` may name.
    ///
    /// Paths overlap when one is a prefix of the other, matching the may-hold
    /// analysis: reading a whole aggregate includes references in its fields.
    pub(crate) fn at(&self, place: &Place) -> OriginSet {
        let mut set = OriginSet::default();
        for (path, origins) in &self.locals[place.local.index() as usize].paths {
            if overlaps(path, &place.projections) {
                set.extend(origins);
            }
        }
        set
    }

    fn assign(&mut self, place: &Place, origins: OriginSet) {
        if place.is_deref() {
            return;
        }
        let paths = &mut self.locals[place.local.index() as usize].paths;
        paths.retain(|(path, _)| !path.starts_with(&place.projections));
        if !origins.is_empty() {
            paths.push((place.projections.clone(), origins));
        }
    }

    fn clear(&mut self, place: &Place) {
        if place.is_deref() {
            return;
        }
        self.locals[place.local.index() as usize]
            .paths
            .retain(|(path, _)| !path.starts_with(&place.projections));
    }

    /// Unions `other` into this state, returning whether anything changed.
    fn join(&mut self, other: &Self) -> bool {
        let mut changed = false;
        for (local, other_local) in other.locals.iter().enumerate() {
            let paths = &mut self.locals[local].paths;
            for (path, origins) in &other_local.paths {
                match paths.iter_mut().find(|(existing, _)| existing == path) {
                    Some((_, existing)) => changed |= existing.extend(origins),
                    None => {
                        paths.push((path.clone(), origins.clone()));
                        changed = true;
                    }
                }
            }
        }
        changed
    }
}

/// The origin state before each statement and after each block.
pub(crate) struct Origins {
    /// `before[block][statement]` is the state before that statement.
    pub(crate) before: Vec<Vec<OriginState>>,
    /// `exit[block]` is the state after the block's last statement.
    pub(crate) exit: Vec<OriginState>,
}

/// Computes the may-name sets.
///
/// A reference parameter starts by naming its own pointee, which is storage
/// outside this frame. Every other local starts empty; definite initialization
/// guarantees it is assigned before it is read.
pub(crate) fn analyze(hir: &Hir, body: &Body) -> Origins {
    let local_count = body.locals.len();
    let mut entry = OriginState::new(local_count);
    for (local, declaration) in body.locals.iter_enumerated() {
        if declaration.kind == LocalKind::Arg
            && matches!(hir.ty_ctxt().kind(declaration.ty), TyKind::Ref { .. })
        {
            entry.assign(&Place::local(local), OriginSet::pointee(local));
        }
    }

    let mut in_states: Vec<Option<OriginState>> = vec![None; body.blocks.len()];
    in_states[body.entry.index() as usize] = Some(entry);
    let mut queue = VecDeque::from([body.entry]);

    while let Some(block) = queue.pop_front() {
        let mut state = in_states[block.index() as usize]
            .clone()
            .expect("a queued block has an incoming state");
        for statement in &body.block(block).statements {
            transfer(hir, body, statement, &mut state);
        }
        for successor in body.block(block).terminator.kind.successors() {
            let slot = &mut in_states[successor.index() as usize];
            let changed = match slot {
                Some(existing) => existing.join(&state),
                None => {
                    *slot = Some(state.clone());
                    true
                }
            };
            if changed {
                queue.push_back(successor);
            }
        }
    }

    let mut before = Vec::with_capacity(body.blocks.len());
    let mut exit = Vec::with_capacity(body.blocks.len());
    for (block_id, block) in body.blocks.iter_enumerated() {
        let mut state = in_states[block_id.index() as usize]
            .clone()
            .unwrap_or_else(|| OriginState::new(local_count));
        let mut row = Vec::with_capacity(block.statements.len());
        for statement in &block.statements {
            row.push(state.clone());
            transfer(hir, body, statement, &mut state);
        }
        before.push(row);
        exit.push(state);
    }

    Origins { before, exit }
}

/// Applies one statement to the may-name state.
fn transfer(hir: &Hir, body: &Body, statement: &Statement, state: &mut OriginState) {
    let StatementKind::Assign { place, rvalue } = &statement.kind;
    if place.is_deref() {
        return;
    }
    let destination_holds_references = hir.ty_ctxt().contains_reference(body.local(place.local).ty);

    match rvalue {
        Rvalue::Ref {
            place: borrowed, ..
        } => {
            let origins = keys_of(state, borrowed);
            state.assign(place, origins);
        }
        Rvalue::SliceRef { base, .. } => {
            let effective = if matches!(
                hir.ty_ctxt().kind(place_ty(hir, body, base)),
                TyKind::Ref { .. }
            ) {
                base.clone().deref()
            } else {
                base.clone()
            };
            let origins = keys_of(state, &effective);
            state.assign(place, origins);
        }
        Rvalue::Len { .. } | Rvalue::Discriminant(_) => state.clear(place),
        Rvalue::Use(operand) => {
            let origins = operand_origins(hir, body, state, operand);
            if let Some(source) = operand.place()
                && matches!(operand, Operand::Move(_))
            {
                state.clear(&source);
            }
            if destination_holds_references {
                state.assign(place, origins);
            } else {
                state.clear(place);
            }
        }
        Rvalue::Call { args, .. } => {
            if destination_holds_references {
                let mut origins = OriginSet::default();
                for arg in args {
                    origins.extend(&operand_origins(hir, body, state, arg));
                }
                state.assign(place, origins);
            } else {
                state.clear(place);
            }
        }
        Rvalue::Aggregate { fields, .. } => {
            state.clear(place);
            if destination_holds_references {
                for (field, operand) in fields {
                    let origins = operand_origins(hir, body, state, operand);
                    if !origins.is_empty() {
                        state.assign(&place.clone().field(*field), origins);
                    }
                }
            }
        }
        Rvalue::Array { elements, .. } => {
            state.clear(place);
            if destination_holds_references {
                let mut origins = OriginSet::default();
                for operand in elements {
                    origins.extend(&operand_origins(hir, body, state, operand));
                }
                state.assign(place, origins);
            }
        }
        Rvalue::Repeat { value, .. } => {
            let origins = operand_origins(hir, body, state, value);
            if destination_holds_references {
                state.assign(place, origins);
            } else {
                state.clear(place);
            }
        }
        Rvalue::Intrinsic { .. } | Rvalue::Binary { .. } | Rvalue::Unary { .. } => {
            state.clear(place)
        }
    }
}

/// Returns the storage the reference held by `operand` may name.
fn operand_origins(hir: &Hir, body: &Body, state: &OriginState, operand: &Operand) -> OriginSet {
    let Some(place) = operand.place() else {
        return OriginSet::default();
    };
    if !matches!(
        hir.ty_ctxt().kind(place_ty(hir, body, &place)),
        TyKind::Ref { .. }
    ) {
        return OriginSet::default();
    }
    if place.is_deref() {
        // The pointer was read out of memory; its target is not tracked.
        return OriginSet::unknown();
    }
    let origins = state.at(&place);
    if origins.is_empty() {
        OriginSet::unknown()
    } else {
        origins
    }
}

/// Returns the storage `place` may name.
///
/// A place without a dereference names frame storage directly. A dereference
/// resolves the reference stored just before it and continues from each storage
/// key that reference may name. A second dereference inside a pointee is not
/// modeled: the checker does not read the contents of borrowed storage.
pub(crate) fn keys_of(state: &OriginState, place: &Place) -> OriginSet {
    let Some(index) = place
        .projections
        .iter()
        .position(|projection| *projection == Projection::Deref)
    else {
        return OriginSet::frame(place.local, field_path(&place.projections));
    };

    let reference = Place::with_projections(place.local, place.projections[..index].to_vec());
    let rest = &place.projections[index + 1..];
    let origins = state.at(&reference);
    if origins.is_unknown() || origins.is_empty() {
        return OriginSet::unknown();
    }

    let mut resolved = OriginSet::default();
    for key in &origins.keys {
        match key {
            StorageKey::Frame { local, fields } => {
                let mut projections: Vec<Projection> = fields
                    .iter()
                    .map(|field| Projection::Field(*field))
                    .collect();
                projections.extend_from_slice(rest);
                let storage = Place::with_projections(*local, projections);
                resolved.extend(&keys_of(state, &storage));
            }
            StorageKey::Pointee { local, fields } => {
                if rest.contains(&Projection::Deref) {
                    resolved.insert_unknown();
                } else {
                    let mut fields = fields.clone();
                    fields.extend(field_path(rest));
                    resolved.insert(StorageKey::Pointee {
                        local: *local,
                        fields,
                    });
                }
            }
        }
    }
    resolved
}

/// Returns the place of the reference a dereference reads through, if the place
/// dereferences anything.
///
/// It is the place just before the first dereference, which names frame storage.
pub(crate) fn first_deref_base(place: &Place) -> Option<Place> {
    let index = place
        .projections
        .iter()
        .position(|projection| *projection == Projection::Deref)?;
    Some(Place::with_projections(
        place.local,
        place.projections[..index].to_vec(),
    ))
}

/// Returns `true` when two sets may name overlapping storage.
pub(crate) fn keys_overlap(left: &OriginSet, right: &OriginSet) -> bool {
    if left.is_unknown() || right.is_unknown() {
        return true;
    }
    left.keys
        .iter()
        .any(|left| right.keys.iter().any(|right| key_overlap(left, right)))
}

fn key_overlap(left: &StorageKey, right: &StorageKey) -> bool {
    match (left, right) {
        (
            StorageKey::Frame {
                local: left_local,
                fields: left_fields,
            },
            StorageKey::Frame {
                local: right_local,
                fields: right_fields,
            },
        )
        | (
            StorageKey::Pointee {
                local: left_local,
                fields: left_fields,
            },
            StorageKey::Pointee {
                local: right_local,
                fields: right_fields,
            },
        ) => left_local == right_local && paths_overlap(left_fields, right_fields),
        _ => false,
    }
}

fn paths_overlap(left: &[DefId], right: &[DefId]) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn overlaps(left: &[Projection], right: &[Projection]) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn field_path(projections: &[Projection]) -> Vec<DefId> {
    projections
        .iter()
        .filter_map(|projection| match projection {
            Projection::Field(field) => Some(*field),
            // A variant field is still a field of the enum's storage.
            Projection::VariantField { field, .. } => Some(*field),
            // Array indices are intentionally absent from storage keys: any
            // element may alias any other when only the runtime value is known,
            // so treating the whole array as one storage key is conservative.
            Projection::Deref | Projection::Index { .. } => None,
        })
        .collect()
}

/// Returns the type of `place`.
pub(crate) fn place_ty(hir: &Hir, body: &Body, place: &Place) -> TyId {
    let mut ty = body.local(place.local).ty;
    for projection in &place.projections {
        ty = match projection {
            Projection::Deref => match hir.ty_ctxt().kind(ty) {
                TyKind::Ref { pointee, .. } => pointee,
                _ => ty,
            },
            Projection::Field(field) => match hir.ty_ctxt().kind(ty) {
                TyKind::Adt(adt) => hir
                    .ty_ctxt()
                    .field(adt, *field)
                    .map_or(ty, |field| field.ty),
                _ => ty,
            },
            Projection::VariantField { variant, field } => match hir.ty_ctxt().kind(ty) {
                TyKind::Adt(adt) => hir
                    .ty_ctxt()
                    .variant_field(adt, *variant, *field)
                    .map_or(ty, |field| field.ty),
                _ => ty,
            },
            Projection::Index { .. } => match hir.ty_ctxt().kind(ty) {
                TyKind::Array { element, .. } => element,
                _ => ty,
            },
        };
    }
    ty
}
