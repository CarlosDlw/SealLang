//! Forward may-hold analysis: which loans each reference local may carry.

use std::collections::VecDeque;

use seal_hash::FxHashSet;
use seal_hir::Hir;
use seal_index::Idx;
use seal_mir::{Body, LocalId, Operand, Rvalue, StatementKind};
use seal_ty::TyKind;

use crate::id::LoanId;
use crate::loan::Loan;

/// A set of loans.
pub(crate) type LoanSet = FxHashSet<LoanId>;

/// The loans each local may hold, indexed by local.
pub(crate) type HoldState = Vec<LoanSet>;

/// The may-hold state before each statement and after each block.
pub(crate) struct Holds {
    /// `before[block][statement]` is the state before that statement.
    pub(crate) before: Vec<Vec<HoldState>>,
    /// `exit[block]` is the state after the block's last statement.
    pub(crate) exit: Vec<HoldState>,
}

/// Computes the may-hold sets.
///
/// The transfer is flow sensitive: assigning a reference to a local replaces
/// its loans, copying a reference copies its loans, and overwriting a reference
/// drops the old ones. A function call returning `*T` is assumed to return a
/// reference derived from its reference arguments; a loaded reference (reading
/// `*p` where the pointee is itself a reference) may alias any loan in the body,
/// because the checker does not model the callee's or the pointee's contents.
pub(crate) fn analyze(
    hir: &Hir,
    body: &Body,
    loans: &[Loan],
    loan_at: &[Vec<Option<LoanId>>],
) -> Holds {
    let local_count = body.locals.len();
    let all_loans: LoanSet = (0..loans.len()).map(LoanId::from_usize).collect();
    let empty_state = || vec![LoanSet::default(); local_count];

    let mut in_states: Vec<Option<HoldState>> = vec![None; body.blocks.len()];
    in_states[body.entry.index() as usize] = Some(empty_state());
    let mut queue = VecDeque::from([body.entry]);

    while let Some(block) = queue.pop_front() {
        let mut state = in_states[block.index() as usize]
            .clone()
            .expect("a queued block has an incoming state");
        let data = body.block(block);
        for (position, statement) in data.statements.iter().enumerate() {
            transfer(
                hir,
                body,
                statement,
                loan_at[block.index() as usize][position],
                &mut state,
                &all_loans,
            );
        }

        for successor in data.terminator.kind.successors() {
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
            .unwrap_or_else(empty_state);
        let mut row = Vec::with_capacity(block.statements.len());
        for (position, statement) in block.statements.iter().enumerate() {
            row.push(state.clone());
            transfer(
                hir,
                body,
                statement,
                loan_at[block_id.index() as usize][position],
                &mut state,
                &all_loans,
            );
        }
        before.push(row);
        exit.push(state);
    }

    Holds { before, exit }
}

/// Unions `other` into `existing`, reporting whether anything changed.
fn join(existing: &mut HoldState, other: &HoldState) -> bool {
    let mut changed = false;
    for (slot, other) in existing.iter_mut().zip(other) {
        for loan in other {
            changed |= slot.insert(*loan);
        }
    }
    changed
}

/// Applies one statement to the may-hold state.
fn transfer(
    hir: &Hir,
    body: &Body,
    statement: &seal_mir::Statement,
    loan: Option<LoanId>,
    state: &mut HoldState,
    all_loans: &LoanSet,
) {
    let StatementKind::Assign { place, rvalue } = &statement.kind;
    if place.is_deref() {
        // Shared references are read-only; a write through one was rejected by
        // type checking and never reaches MIR.
        return;
    }
    let destination = place.local.index() as usize;

    match rvalue {
        Rvalue::Ref(_) => {
            state[destination].clear();
            if let Some(loan) = loan {
                state[destination].insert(loan);
            }
        }
        Rvalue::Use(operand) => {
            if let Some(source) = operand.place() {
                if source.is_deref() {
                    state[destination].clear();
                    if is_reference(hir, body, place.local) {
                        state[destination].extend(all_loans.iter().copied());
                    }
                } else {
                    let moved = matches!(operand, Operand::Move(_));
                    let carried = state[source.local.index() as usize].clone();
                    state[destination] = carried;
                    if moved {
                        state[source.local.index() as usize].clear();
                    }
                }
            } else {
                state[destination].clear();
            }
        }
        Rvalue::Call { args, .. } => {
            state[destination].clear();
            if is_reference(hir, body, place.local) {
                for arg in args {
                    let Some(source) = arg.place() else {
                        continue;
                    };
                    if source.is_deref() {
                        state[destination].extend(all_loans.iter().copied());
                    } else {
                        let carried = state[source.local.index() as usize].clone();
                        state[destination].extend(carried);
                    }
                }
            }
        }
        Rvalue::Binary { .. } | Rvalue::Unary { .. } => state[destination].clear(),
    }
}

/// Returns `true` if the local's declared type is a reference.
fn is_reference(hir: &Hir, body: &Body, local: LocalId) -> bool {
    matches!(hir.ty_ctxt().kind(body.local(local).ty), TyKind::Ref(_))
}
