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

use seal_const_eval::{AllocId, AllocSource, ConstProgram, ConstValue};
use seal_def::DefId;
use seal_hash::{FxHashMap, FxHashSet};
use seal_hir::Hir;
use seal_mir::{
    Body, Constant, LocalId, LocalKind, Operand, Place, PlaceRoot, Projection, Rvalue, Statement,
    StatementKind,
};
use seal_ty::{TyId, TyKind};

/// Locals that hold a materialized constant value.
///
/// Lowering gives a const place no root of its own: it assigns the evaluated
/// value to a fresh temporary. Recording that binding lets a borrow of the
/// temporary name the const's program-lifetime allocation instead of frame
/// storage that dies with the function.
pub(crate) type ConstLocals = FxHashMap<LocalId, DefId>;

/// Returns the locals that are assigned exactly once, from `Operand::Const`.
///
/// A local written by anything else is omitted: it may hold an ordinary value,
/// so treating its storage as global would hide a dangling reference.
pub(crate) fn const_locals(body: &Body) -> ConstLocals {
    let mut constants = ConstLocals::default();
    let mut shadowed = FxHashSet::default();
    for block in body.blocks.iter() {
        for statement in &block.statements {
            let StatementKind::Assign { place, rvalue } = &statement.kind else {
                continue;
            };
            let PlaceRoot::Local(local) = place.root else {
                continue;
            };
            // A source `let` binding is frame storage even when its value came
            // from a const; only a compiler materialization is program lifetime.
            if body.local(local).kind != LocalKind::Temp {
                continue;
            }
            if shadowed.contains(&local) {
                continue;
            }
            if place.projections.is_empty()
                && let Rvalue::Use(Operand::Const(def)) = rvalue
                && !constants.contains_key(&local)
            {
                constants.insert(local, *def);
                continue;
            }
            shadowed.insert(local);
            constants.remove(&local);
        }
    }
    constants
}

/// One owned-storage component below a local.
///
/// Unlike an aggregate field, a tuple element has no definition, so it is
/// addressed by position. Keeping both forms in one path is what lets two
/// references to different tuple elements stay disjoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum StorageField {
    /// A named struct or enum field.
    Field(DefId),
    /// A tuple element by zero-based position.
    TupleField(u32),
}

/// A storage location a reference may point at.
///
/// A key names frame storage (`Frame`), the pointee of a reference whose target
/// lies outside the frame (`Pointee`), or the allocation of a global item
/// (`Static`, `Const`). The variants never overlap: a local's storage, a
/// pointer's target, and a global allocation are distinct places.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum StorageKey {
    /// Storage of a local path in this frame.
    Frame {
        /// The local that owns the storage.
        local: LocalId,
        /// The field path below the local; empty for the whole local.
        fields: Vec<StorageField>,
    },
    /// Storage reachable through a reference whose target is outside this frame.
    Pointee {
        /// The reference local that names the storage.
        local: LocalId,
        /// The field path below the pointee.
        fields: Vec<StorageField>,
    },
    /// The program-lifetime allocation of a static.
    Static {
        /// The static definition.
        def: DefId,
        /// The field path below the static; empty for the whole allocation.
        fields: Vec<StorageField>,
    },
    /// The program-lifetime allocation of an addressed const or promotion.
    Const {
        /// The const definition, or the definition owning a promotion.
        def: DefId,
        /// The field path below the allocation; empty for the whole value.
        fields: Vec<StorageField>,
    },
    /// The immutable, program-lifetime bytes of a string literal.
    String,
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

    fn frame(local: LocalId, fields: Vec<StorageField>) -> Self {
        let mut set = Self::default();
        set.insert(StorageKey::Frame { local, fields });
        set
    }

    fn global(key: StorageKey) -> Self {
        let mut set = Self::default();
        set.insert(key);
        set
    }

    /// Returns the program-lifetime allocations this set may name.
    pub(crate) fn globals(&self) -> impl Iterator<Item = StorageKey> + '_ {
        self.keys.iter().filter_map(|key| match key {
            StorageKey::Static { .. } | StorageKey::Const { .. } | StorageKey::String => {
                Some(key.clone())
            }
            StorageKey::Frame { .. } | StorageKey::Pointee { .. } => None,
        })
    }

    /// Returns `true` when every known target is a program-lifetime allocation.
    ///
    /// Such a reference outlives the frame, including a string literal. An
    /// unknown target does not qualify: it may still name frame storage.
    pub(crate) fn only_globals(&self) -> bool {
        !self.unknown
            && !self.keys.is_empty()
            && self.keys.iter().all(|key| {
                matches!(
                    key,
                    StorageKey::Static { .. } | StorageKey::Const { .. } | StorageKey::String
                )
            })
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
        let Some(local) = place.local_id() else {
            return OriginSet::unknown();
        };
        let mut set = OriginSet::default();
        for (path, origins) in &self.locals[local.index() as usize].paths {
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
        let Some(local) = place.local_id() else {
            return;
        };
        let paths = &mut self.locals[local.index() as usize].paths;
        paths.retain(|(path, _)| !path.starts_with(&place.projections));
        if !origins.is_empty() {
            paths.push((place.projections.clone(), origins));
        }
    }

    fn clear(&mut self, place: &Place) {
        if place.is_deref() {
            return;
        }
        let Some(local) = place.local_id() else {
            return;
        };
        self.locals[local.index() as usize]
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
pub(crate) fn analyze(
    hir: &Hir,
    body: &Body,
    program: Option<&ConstProgram>,
    const_locals: &ConstLocals,
) -> Origins {
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
            transfer(hir, body, program, const_locals, statement, &mut state);
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
            transfer(hir, body, program, const_locals, statement, &mut state);
        }
        before.push(row);
        exit.push(state);
    }

    Origins { before, exit }
}

/// Applies one statement to the may-name state.
fn transfer(
    hir: &Hir,
    body: &Body,
    program: Option<&ConstProgram>,
    const_locals: &ConstLocals,
    statement: &Statement,
    state: &mut OriginState,
) {
    let (place, rvalue) = match &statement.kind {
        StatementKind::Assign { place, rvalue } => (place, rvalue),
        // A dropped place no longer reaches any storage, so the origins it
        // carried are gone.
        StatementKind::Drop { place, .. } | StatementKind::DropRemaining { place, .. } => {
            state.clear(place);
            return;
        }
    };
    if place.is_deref() {
        return;
    }
    let destination_holds_references = place
        .local_id()
        .is_some_and(|local| hir.ty_ctxt().contains_reference(body.local(local).ty));

    match rvalue {
        Rvalue::Ref {
            place: borrowed, ..
        } => {
            let origins = keys_of(state, borrowed, const_locals);
            state.assign(place, origins);
        }
        Rvalue::SliceRef { base, .. } | Rvalue::StrView { base, .. } => {
            let effective = if matches!(
                hir.ty_ctxt().kind(place_ty(hir, body, base)),
                TyKind::Ref { .. }
            ) {
                base.clone().deref()
            } else {
                base.clone()
            };
            let origins = keys_of(state, &effective, const_locals);
            state.assign(place, origins);
        }
        Rvalue::MakeTraitObject { source, .. } => {
            let origins = operand_origins(hir, body, program, state, source);
            if destination_holds_references {
                state.assign(place, origins);
            } else {
                state.clear(place);
            }
        }
        Rvalue::TraitObjectReborrow {
            place: borrowed, ..
        } => {
            let origins = keys_of(state, borrowed, const_locals);
            state.assign(place, origins);
        }
        Rvalue::DynamicCall { receiver, args, .. } => {
            if destination_holds_references {
                let mut origins = operand_origins(hir, body, program, state, receiver);
                for arg in args {
                    origins.extend(&operand_origins(hir, body, program, state, arg));
                }
                state.assign(place, origins);
            } else {
                state.clear(place);
            }
        }
        Rvalue::Len { .. } | Rvalue::Discriminant(_) | Rvalue::TypeInfo { .. } => {
            state.clear(place)
        }
        Rvalue::Cast { operand, .. } => {
            // A pointer cast keeps the pointee it came from: `&value as *raw T`
            // still names `value`, so a loan through it conflicts with writes to
            // the same storage. Numeric casts carry no origins.
            let carries_pointer = place.local_id().is_some_and(|local| {
                let ty = body.local(local).ty;
                hir.ty_ctxt().contains_reference(ty) || hir.ty_ctxt().contains_raw_pointer(ty)
            });
            if carries_pointer {
                let origins = operand_origins(hir, body, program, state, operand);
                state.assign(place, origins);
            } else {
                state.clear(place);
            }
        }
        Rvalue::Use(operand) => {
            // A string literal is a pointer to immutable, program-lifetime
            // data. It has no allocation ID in `ConstValue`, so give it a
            // global key instead of treating the loan as unknown: an unknown
            // target would make every later temporary write conflict.
            let origins = if matches!(operand, Operand::Constant(Constant::Str { .. })) {
                OriginSet::global(StorageKey::String)
            } else {
                operand_origins(hir, body, program, state, operand)
            };
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
                    origins.extend(&operand_origins(hir, body, program, state, arg));
                }
                state.assign(place, origins);
            } else {
                state.clear(place);
            }
        }
        Rvalue::IndirectCall { callee, args, .. } => {
            if destination_holds_references {
                let mut origins = operand_origins(hir, body, program, state, callee);
                for arg in args {
                    origins.extend(&operand_origins(hir, body, program, state, arg));
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
                    let origins = operand_origins(hir, body, program, state, operand);
                    if !origins.is_empty() {
                        state.assign(&place.clone().field(*field), origins);
                    }
                }
            }
        }
        Rvalue::Tuple { elements, .. } => {
            state.clear(place);
            if destination_holds_references {
                for (index, operand) in elements.iter().enumerate() {
                    let origins = operand_origins(hir, body, program, state, operand);
                    if !origins.is_empty() {
                        state.assign(&place.clone().tuple_field(index as u32), origins);
                    }
                }
            }
        }
        Rvalue::Array { elements, .. } => {
            state.clear(place);
            if destination_holds_references {
                let mut origins = OriginSet::default();
                for operand in elements {
                    origins.extend(&operand_origins(hir, body, program, state, operand));
                }
                state.assign(place, origins);
            }
        }
        Rvalue::Repeat { value, .. } => {
            let origins = operand_origins(hir, body, program, state, value);
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
fn operand_origins(
    hir: &Hir,
    body: &Body,
    program: Option<&ConstProgram>,
    state: &OriginState,
    operand: &Operand,
) -> OriginSet {
    if let Operand::Const(def) = operand {
        return match program.and_then(|program| program.value(*def).map(|value| (program, value))) {
            Some((program, value)) => value_origins(program, value),
            None => OriginSet::unknown(),
        };
    }
    let Some(place) = operand.place() else {
        return OriginSet::default();
    };
    if let PlaceRoot::Static(def) | PlaceRoot::Const(def) = place.root
        && matches!(
            hir.ty_ctxt().kind(place_ty(hir, body, &place)),
            TyKind::Ref { .. }
        )
    {
        return match program.and_then(|program| program.value(def).map(|value| (program, value))) {
            Some((program, value)) => value_origins(program, value),
            None => OriginSet::unknown(),
        };
    }
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

/// Returns the global storage the references inside `value` may name.
///
/// References in an evaluated constant always point at compiler-owned
/// allocations. Mapping the allocation source to a storage key lets a copied
/// constant participate in the same overlap rules as a local borrow.
fn value_origins(program: &ConstProgram, value: &ConstValue) -> OriginSet {
    match value {
        ConstValue::Ref { target, .. }
        | ConstValue::Slice { target, .. }
        | ConstValue::RawPtr { target, .. }
        | ConstValue::RawSlice { target, .. } => allocation_origins(program, *target),
        ConstValue::Dyn { data, .. } | ConstValue::RawDyn { data, .. } => {
            allocation_origins(program, *data)
        }
        ConstValue::Tuple(elements) | ConstValue::Array(elements) => {
            let mut origins = OriginSet::default();
            for element in elements {
                origins.extend(&value_origins(program, element));
            }
            origins
        }
        ConstValue::Struct { fields, .. } | ConstValue::Variant { fields, .. } => {
            let mut origins = OriginSet::default();
            for (_, field) in fields {
                origins.extend(&value_origins(program, field));
            }
            origins
        }
        ConstValue::Int { .. }
        | ConstValue::Float { .. }
        | ConstValue::Bool(_)
        | ConstValue::Char(_)
        | ConstValue::Unit
        | ConstValue::Str { .. }
        | ConstValue::RawStr { .. }
        | ConstValue::RawStrData { .. } => OriginSet::default(),
    }
}

fn allocation_origins(program: &ConstProgram, target: AllocId) -> OriginSet {
    match program.allocation(target).source {
        AllocSource::Static(def) => OriginSet::global(StorageKey::Static {
            def,
            fields: Vec::new(),
        }),
        AllocSource::Const(def) => OriginSet::global(StorageKey::Const {
            def,
            fields: Vec::new(),
        }),
        AllocSource::Promoted { owner, .. } => OriginSet::global(StorageKey::Const {
            def: owner,
            fields: Vec::new(),
        }),
        // A reference to a const-function frame cannot appear in a finished
        // constant; reporting it as unknown keeps the checker conservative if
        // a future evaluator ever produces one.
        AllocSource::ConstFnLocal { .. } => OriginSet::unknown(),
    }
}

/// Returns the storage `place` may name.
///
/// A place without a dereference names frame storage directly. A dereference
/// resolves the reference stored just before it and continues from each storage
/// key that reference may name. A second dereference inside a pointee is not
/// modeled: the checker does not read the contents of borrowed storage.
pub(crate) fn keys_of(state: &OriginState, place: &Place, const_locals: &ConstLocals) -> OriginSet {
    let Some((index, raw)) =
        place
            .projections
            .iter()
            .enumerate()
            .find_map(|(index, projection)| match projection {
                Projection::Deref => Some((index, false)),
                Projection::RawDeref => Some((index, true)),
                _ => None,
            })
    else {
        return match place.root {
            PlaceRoot::Local(local) => match const_locals.get(&local) {
                Some(def) => OriginSet::global(StorageKey::Const {
                    def: *def,
                    fields: field_path(&place.projections),
                }),
                None => OriginSet::frame(local, field_path(&place.projections)),
            },
            PlaceRoot::Static(def) => OriginSet::global(StorageKey::Static {
                def,
                fields: field_path(&place.projections),
            }),
            PlaceRoot::Const(def) => OriginSet::global(StorageKey::Const {
                def,
                fields: field_path(&place.projections),
            }),
        };
    };

    let Some(base) = place.local_id() else {
        return OriginSet::unknown();
    };
    let reference = Place::with_projections(base, place.projections[..index].to_vec());
    let rest = &place.projections[index + 1..];
    let origins = state.at(&reference);
    if origins.is_unknown() {
        return OriginSet::unknown();
    }
    if origins.is_empty() {
        // A raw pointer with no known target (a parameter, or one built from an
        // integer) still names one external pointee per pointer local. Keying it
        // separately keeps a loan through it from colliding with every frame
        // local, while accesses through the same pointer stay conservative.
        if raw
            && !rest
                .iter()
                .any(|projection| matches!(projection, Projection::Deref | Projection::RawDeref))
        {
            return OriginSet::global(StorageKey::Pointee {
                local: base,
                fields: field_path(rest),
            });
        }
        return OriginSet::unknown();
    }

    let mut resolved = OriginSet::default();
    for key in &origins.keys {
        match key {
            StorageKey::Frame { local, fields } => {
                let mut projections: Vec<Projection> = fields
                    .iter()
                    .map(|field| match field {
                        StorageField::Field(field) => Projection::Field(*field),
                        StorageField::TupleField(index) => Projection::TupleField(*index),
                    })
                    .collect();
                projections.extend_from_slice(rest);
                let storage = Place::with_projections(*local, projections);
                resolved.extend(&keys_of(state, &storage, const_locals));
            }
            StorageKey::Pointee { local, fields } => {
                if rest.iter().any(|projection| {
                    matches!(projection, Projection::Deref | Projection::RawDeref)
                }) {
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
            StorageKey::Static { def, fields } => {
                if rest.iter().any(|projection| {
                    matches!(projection, Projection::Deref | Projection::RawDeref)
                }) {
                    resolved.insert_unknown();
                } else {
                    let mut fields = fields.clone();
                    fields.extend(field_path(rest));
                    resolved.insert(StorageKey::Static { def: *def, fields });
                }
            }
            StorageKey::Const { def, fields } => {
                if rest.iter().any(|projection| {
                    matches!(projection, Projection::Deref | Projection::RawDeref)
                }) {
                    resolved.insert_unknown();
                } else {
                    let mut fields = fields.clone();
                    fields.extend(field_path(rest));
                    resolved.insert(StorageKey::Const { def: *def, fields });
                }
            }
            StorageKey::String => {
                if rest.iter().any(|projection| {
                    matches!(projection, Projection::Deref | Projection::RawDeref)
                }) {
                    resolved.insert_unknown();
                } else {
                    resolved.insert(StorageKey::String);
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
        place.local_id()?,
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
        (
            StorageKey::Static {
                def: left_def,
                fields: left_fields,
            },
            StorageKey::Static {
                def: right_def,
                fields: right_fields,
            },
        )
        | (
            StorageKey::Const {
                def: left_def,
                fields: left_fields,
            },
            StorageKey::Const {
                def: right_def,
                fields: right_fields,
            },
        ) => left_def == right_def && paths_overlap(left_fields, right_fields),
        (StorageKey::String, StorageKey::String) => true,
        _ => false,
    }
}

fn paths_overlap(left: &[StorageField], right: &[StorageField]) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn overlaps(left: &[Projection], right: &[Projection]) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn field_path(projections: &[Projection]) -> Vec<StorageField> {
    projections
        .iter()
        .filter_map(|projection| match projection {
            Projection::Field(field) => Some(StorageField::Field(*field)),
            // A variant field is still a field of the enum's storage.
            Projection::VariantField { field, .. } => Some(StorageField::Field(*field)),
            Projection::TupleField(index) => Some(StorageField::TupleField(*index)),
            // Array indices are intentionally absent from storage keys: any
            // element may alias any other when only the runtime value is known,
            // so treating the whole array as one storage key is conservative.
            Projection::Deref | Projection::RawDeref | Projection::Index { .. } => None,
        })
        .collect()
}

/// Returns the type of `place`.
pub(crate) fn place_ty(hir: &Hir, body: &Body, place: &Place) -> TyId {
    let mut ty = match place.root {
        seal_mir::PlaceRoot::Local(local) => body.local(local).ty,
        seal_mir::PlaceRoot::Static(def) | seal_mir::PlaceRoot::Const(def) => hir
            .body_of(def)
            .and_then(|body| body.ret)
            .unwrap_or_else(|| hir.ty_ctxt().error_ty()),
    };
    for projection in &place.projections {
        ty = match projection {
            Projection::Deref => match hir.ty_ctxt().kind(ty) {
                TyKind::Ref { pointee, .. } => pointee,
                _ => ty,
            },
            Projection::RawDeref => match hir.ty_ctxt().kind(ty) {
                TyKind::RawPtr { pointee, .. } => pointee,
                _ => ty,
            },
            Projection::Field(field) => match hir.ty_ctxt().kind(ty) {
                TyKind::Adt { adt, args } => {
                    hir.ty_ctxt().field_ty(adt, args, *field).unwrap_or(ty)
                }
                _ => ty,
            },
            Projection::VariantField { variant, field } => match hir.ty_ctxt().kind(ty) {
                TyKind::Adt { adt, args } => hir
                    .ty_ctxt()
                    .variant_field_ty(adt, args, *variant, *field)
                    .unwrap_or(ty),
                _ => ty,
            },
            Projection::TupleField(index) => match hir.ty_ctxt().kind(ty) {
                TyKind::Tuple(tuple) => hir
                    .ty_ctxt()
                    .tuple_element(tuple, *index as usize)
                    .unwrap_or(ty),
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
