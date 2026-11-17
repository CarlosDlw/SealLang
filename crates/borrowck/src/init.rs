//! Forward initialization analysis over move paths.

use std::collections::VecDeque;

use seal_mir::{Body, LocalId, LocalKind, Operand, Place, Rvalue, Statement, StatementKind};
use seal_ty::{InitPaths, InitState, PathElem, TyCtxt, TyId};

/// The initialization state of a place, as message selection needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Init {
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
pub(crate) struct InitInfo {
    /// `before[block][statement]` is the state before that statement.
    pub(crate) before: Vec<Vec<InitPaths<LocalId>>>,
    /// `exit[block]` is the state after the block's last statement.
    pub(crate) exit: Vec<InitPaths<LocalId>>,
}

/// Computes definite initialization.
///
/// The lattice joins with "initialized on every path". Parameters start
/// initialized, every other local starts uninitialized, and an unreachable
/// block starts at the top element so dead code never reports a false error.
pub(crate) fn analyze(body: &Body) -> InitInfo {
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
            transfer(statement, &mut state);
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
            transfer(statement, &mut state);
        }
        before.push(row);
        exit.push(state);
    }

    InitInfo { before, exit }
}

/// Applies one statement: moves happen during evaluation, then the destination
/// is initialized.
fn transfer(statement: &Statement, state: &mut InitPaths<LocalId>) {
    let StatementKind::Assign { place, rvalue } = &statement.kind;
    match rvalue {
        Rvalue::Use(operand) => move_operand(operand, state),
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
            if let seal_mir::IntrinsicReceiver::Value(operand) = receiver {
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
        Rvalue::Array { elements, .. } => {
            for element in elements {
                move_operand(element, state);
            }
        }
        Rvalue::Repeat { value, .. } => move_operand(value, state),
        Rvalue::Len { slice } => move_operand(slice, state),
        Rvalue::Discriminant(_) | Rvalue::Ref { .. } | Rvalue::SliceRef { .. } => {}
    }
    if let Some((local, fields)) = path_of(place) {
        state.assign(local, &fields);
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
fn path_of(place: &Place) -> Option<(LocalId, Vec<PathElem>)> {
    let mut path = Vec::new();
    for projection in &place.projections {
        match projection {
            seal_mir::Projection::Deref => return None,
            seal_mir::Projection::Field(field)
            | seal_mir::Projection::VariantField { field, .. } => {
                path.push(PathElem::Field(*field))
            }
            seal_mir::Projection::Index {
                constant: Some(index),
                ..
            } => path.push(PathElem::Index(*index)),
            seal_mir::Projection::Index { constant: None, .. } => break,
        }
    }
    Some((place.local, path))
}

/// Returns the state of a place for message selection.
pub(crate) fn state_of(
    init: &InitPaths<LocalId>,
    place: &Place,
    tcx: &TyCtxt,
    root_ty: TyId,
) -> Init {
    if place.is_deref() {
        return Init::Initialized;
    }
    let mut path = Vec::new();
    for projection in &place.projections {
        match projection {
            seal_mir::Projection::Deref => return Init::Initialized,
            seal_mir::Projection::Field(field)
            | seal_mir::Projection::VariantField { field, .. } => {
                path.push(PathElem::Field(*field))
            }
            seal_mir::Projection::Index {
                constant: Some(index),
                ..
            } => path.push(PathElem::Index(*index)),
            seal_mir::Projection::Index { constant: None, .. } => break,
        }
    }
    match init.state(place.local, &path, tcx, root_ty) {
        InitState::Initialized => Init::Initialized,
        InitState::Moved => Init::Moved,
        InitState::PartiallyMoved => Init::PartiallyMoved,
        InitState::Uninitialized | InitState::PartiallyInitialized => Init::Uninitialized,
    }
}
