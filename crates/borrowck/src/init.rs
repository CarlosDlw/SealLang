//! Forward initialization analysis.

use std::collections::VecDeque;

use seal_mir::{Body, LocalId, LocalKind, Operand, Place, Rvalue, Statement, StatementKind};

/// The initialization state of one local at one point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Init {
    /// Storage exists but no value was written.
    Uninitialized,
    /// A value is present.
    Initialized,
    /// A value was present and has been moved out.
    Moved,
}

/// The initialization state of every local.
pub(crate) type InitState = Vec<Init>;

/// The state before each statement and after each block.
pub(crate) struct InitInfo {
    /// `before[block][statement]` is the state before that statement.
    pub(crate) before: Vec<Vec<InitState>>,
    /// `exit[block]` is the state after the block's last statement.
    pub(crate) exit: Vec<InitState>,
}

/// Computes definite initialization.
///
/// The lattice joins with "initialized on every path": two different states
/// join to uninitialized, because a read would not be safe on both. Parameters
/// start initialized, every other local starts uninitialized, and an unreachable
/// block starts at the top element so dead code never reports a false error.
pub(crate) fn analyze(body: &Body) -> InitInfo {
    let local_count = body.locals.len();
    let top = vec![Init::Initialized; local_count];
    let entry: InitState = body
        .locals
        .iter()
        .map(|declaration| {
            if declaration.kind == LocalKind::Arg {
                Init::Initialized
            } else {
                Init::Uninitialized
            }
        })
        .collect();

    let mut in_states: Vec<Option<InitState>> = vec![None; body.blocks.len()];
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
                Some(existing) => join(existing, &state),
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

/// Joins two states elementwise, keeping only what is initialized on both paths.
fn join(existing: &mut InitState, other: &InitState) -> bool {
    let mut changed = false;
    for (slot, other) in existing.iter_mut().zip(other) {
        let joined = if *slot == *other {
            *slot
        } else {
            Init::Uninitialized
        };
        if *slot != joined {
            *slot = joined;
            changed = true;
        }
    }
    changed
}

/// Applies one statement: moves happen during evaluation, then the destination
/// is initialized.
fn transfer(statement: &Statement, state: &mut InitState) {
    let StatementKind::Assign { place, rvalue } = &statement.kind;
    match rvalue {
        Rvalue::Use(operand) => move_operand(*operand, state),
        Rvalue::Binary { lhs, rhs, .. } => {
            move_operand(*lhs, state);
            move_operand(*rhs, state);
        }
        Rvalue::Unary { operand, .. } => move_operand(*operand, state),
        Rvalue::Call { args, .. } => {
            for arg in args {
                move_operand(*arg, state);
            }
        }
        Rvalue::Ref(_) => {}
    }
    if !place.is_deref() {
        state[place.local.index() as usize] = Init::Initialized;
    }
}

fn move_operand(operand: Operand, state: &mut InitState) {
    if let Operand::Move(place) = operand {
        mark_moved(place, state);
    }
}

fn mark_moved(place: Place, state: &mut InitState) {
    if !place.is_deref() {
        state[place.local.index() as usize] = Init::Moved;
    }
}

/// Returns the state of a local for message selection.
pub(crate) fn state_of(state: &InitState, local: LocalId) -> Init {
    state[local.index() as usize]
}
