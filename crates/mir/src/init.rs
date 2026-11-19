//! Forward initialization analysis over move paths.
//!
//! Drop elaboration and borrow checking both need to know whether a place
//! definitely holds a value at a program point. The facts live in MIR rather
//! than in the checker because they are derived from the lowered statements:
//! drop elaboration decides between an unconditional drop and a guarded one
//! with the same analysis the checker uses for move diagnostics, so the two
//! passes cannot disagree about what is initialized.

use std::collections::VecDeque;

use seal_hir::Hir;
use seal_ty::{InitPaths, InitState, ParamEnv, PathElem, TyCtxt, TyId, place_type};

use crate::body::{Body, LocalKind};
use crate::id::LocalId;
use crate::operand::Operand;
use crate::place::{Place, PlaceRoot, Projection};
use crate::rvalue::{IntrinsicReceiver, Rvalue};
use crate::statement::{DropKind, Statement, StatementKind};

/// The initialization state of a place, as message selection needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Init {
    /// No value is present.
    Uninitialized,
    /// A value is present.
    Initialized,
    /// A value was present and has been moved out.
    Moved,
    /// The place is intact except for at least one field that was moved out.
    PartiallyMoved,
}

/// The state before each statement and after each block.
pub struct InitInfo {
    /// `before[block][statement]` is the state before that statement.
    pub before: Vec<Vec<InitPaths<LocalId>>>,
    /// `exit[block]` is the state after the block's last statement.
    pub exit: Vec<InitPaths<LocalId>>,
}

/// Computes definite initialization.
///
/// The lattice joins with "initialized on every path". Parameters start
/// initialized, every other local starts uninitialized, and an unreachable
/// block starts at the top element so dead code never reports a false error.
pub fn analyze(hir: &Hir, body: &Body) -> InitInfo {
    let context = Context {
        tcx: hir.ty_ctxt(),
        body,
        param_env: hir.param_env(body.owner),
    };
    let mut entry = InitPaths::new();
    for (local, declaration) in body.locals.iter_enumerated() {
        if declaration.kind == LocalKind::Arg {
            entry.assign(local, &[]);
        }
    }
    let mut top = InitPaths::new();
    for local in body.locals.iter_enumerated().map(|(local, _)| local) {
        top.assign(local, &[]);
    }

    let mut in_states: Vec<Option<InitPaths<LocalId>>> = vec![None; body.blocks.len()];
    in_states[body.entry.index() as usize] = Some(entry);
    let mut queue = VecDeque::from([body.entry]);

    while let Some(block) = queue.pop_front() {
        let mut state = in_states[block.index() as usize]
            .clone()
            .expect("a queued block has an incoming state");
        for statement in &body.block(block).statements {
            transfer(&context, statement, &mut state);
        }

        for successor in body.block(block).terminator.kind.successors() {
            let slot = &mut in_states[successor.index() as usize];
            let changed = match slot {
                Some(existing) => {
                    let before = existing.clone();
                    existing.join(&state);
                    *existing != before
                }
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
            .unwrap_or_else(|| top.clone());
        let mut row = Vec::with_capacity(block.statements.len());
        for statement in &block.statements {
            row.push(state.clone());
            transfer(&context, statement, &mut state);
        }
        before.push(row);
        exit.push(state);
    }

    InitInfo { before, exit }
}

/// The type information the initialization analysis needs.
struct Context<'a> {
    tcx: &'a TyCtxt,
    body: &'a Body,
    /// Resolves type parameters, which decides whether a drop consumes a value.
    param_env: ParamEnv,
}

/// Applies one statement: moves happen during evaluation, then the destination
/// is initialized.
fn transfer(context: &Context<'_>, statement: &Statement, state: &mut InitPaths<LocalId>) {
    let (place, rvalue) = match &statement.kind {
        StatementKind::Assign { place, rvalue } => (place, rvalue),
        StatementKind::Drop { place, kind } => {
            if *kind != DropKind::Overwrite {
                drop_place(context, place, state);
            }
            return;
        }
        StatementKind::DropRemaining { place, from } => {
            move_operand(from, state);
            drop_place(context, place, state);
            return;
        }
    };
    match rvalue {
        Rvalue::Use(operand) | Rvalue::Cast { operand, .. } => move_operand(operand, state),
        Rvalue::Binary { lhs, rhs, .. } => {
            move_operand(lhs, state);
            move_operand(rhs, state);
        }
        Rvalue::Unary { operand, .. } => move_operand(operand, state),
        Rvalue::Call { args, .. } => {
            for arg in args {
                move_operand(arg, state);
            }
        }
        Rvalue::Intrinsic { receiver, args, .. } => {
            if let IntrinsicReceiver::Value(operand) = receiver {
                move_operand(operand, state);
            }
            for arg in args {
                move_operand(arg, state);
            }
        }
        Rvalue::Aggregate { fields, .. } => {
            for (_, operand) in fields {
                move_operand(operand, state);
            }
        }
        Rvalue::Tuple { elements, .. } | Rvalue::Array { elements, .. } => {
            for element in elements {
                move_operand(element, state);
            }
        }
        Rvalue::Repeat { value, .. } => move_operand(value, state),
        Rvalue::Len { slice } => move_operand(slice, state),
        Rvalue::MakeTraitObject { source, .. } => move_operand(source, state),
        Rvalue::TraitObjectReborrow { source, .. } => move_operand(source, state),
        Rvalue::DynamicCall { receiver, args, .. } => {
            move_operand(receiver, state);
            for arg in args {
                move_operand(arg, state);
            }
        }
        Rvalue::Discriminant(_) | Rvalue::Ref { .. } | Rvalue::SliceRef { .. } => {}
    }
    if let Some((local, fields)) = path_of(place) {
        state.assign(local, &fields);
    }
}

/// Marks a dropped place as moved.
///
/// A drop consumes the value, so the place is dead afterwards. A `Copy` value is
/// not moved out by a drop, exactly like any other use, and a place reached
/// through a reference says nothing about this frame's storage.
fn drop_place(context: &Context<'_>, place: &Place, state: &mut InitPaths<LocalId>) {
    let Some((local, fields)) = path_of(place) else {
        return;
    };
    let root = context.body.local(local).ty;
    let Some(ty) = place_type(context.tcx, root, &fields) else {
        return;
    };
    if !context.tcx.is_copy_with(ty, &context.param_env) {
        state.move_out(local, &fields);
    }
}

fn move_operand(operand: &Operand, state: &mut InitPaths<LocalId>) {
    if let Operand::Move(place) = operand
        && let Some((local, fields)) = path_of(place)
    {
        state.move_out(local, &fields);
    }
}

/// Splits a place into its root local and its field path, if it stays in this
/// frame; reads through a reference say nothing about initialization here.
///
/// Drop elaboration uses the same split for flag keys and for the maintenance
/// of their values, so both passes name a place the same way.
pub fn path_of(place: &Place) -> Option<(LocalId, Vec<PathElem>)> {
    let mut path = Vec::new();
    for projection in &place.projections {
        match projection {
            Projection::Deref | Projection::RawDeref => return None,
            Projection::Field(field) | Projection::VariantField { field, .. } => {
                path.push(PathElem::Field(*field))
            }
            Projection::TupleField(index) => path.push(PathElem::TupleField(*index)),
            Projection::Index {
                constant: Some(index),
                ..
            } => path.push(PathElem::Index(*index)),
            Projection::Index { constant: None, .. } => break,
        }
    }
    Some((place.local_id()?, path))
}

/// Returns the state of a place for message selection.
pub fn state_of(init: &InitPaths<LocalId>, place: &Place, tcx: &TyCtxt, root_ty: TyId) -> Init {
    // A static lives for the whole program and a const is materialized before
    // use, so neither can be uninitialized or moved from.
    if place.is_deref() || !matches!(place.root, PlaceRoot::Local(_)) {
        return Init::Initialized;
    }
    let Some((local, path)) = path_of(place) else {
        return Init::Initialized;
    };
    match init.state(local, &path, tcx, root_ty) {
        InitState::Initialized => Init::Initialized,
        InitState::Moved => Init::Moved,
        InitState::PartiallyMoved => Init::PartiallyMoved,
        InitState::Uninitialized | InitState::PartiallyInitialized => Init::Uninitialized,
    }
}
