//! Forward may-hold analysis: which loans each place may carry.
//!
//! Loans are tracked per path, because a reference stored in `p.r` must stay
//! alive while `p.s` is written, and a reference field of an aggregate moves
//! with the aggregate. The transfer is flow sensitive: assigning a reference
//! replaces the loans of its path, copying a reference copies its loans, and
//! overwriting storage drops the loans it carried.

use std::collections::VecDeque;

use seal_hash::FxHashSet;
use seal_hir::Hir;
use seal_index::Idx;
use seal_mir::{Body, LocalId, Operand, Place, Projection, Rvalue, StatementKind};

use crate::id::LoanId;
use crate::loan::Loan;

/// A set of loans.
pub(crate) type LoanSet = FxHashSet<LoanId>;

/// The loans a local's paths carry.
#[derive(Clone, Debug, Default)]
struct LocalHolds {
    /// Exact paths and the loans they carry, in insertion order.
    paths: Vec<(Vec<Projection>, LoanSet)>,
}

/// The loans every place may hold.
#[derive(Clone, Debug)]
pub(crate) struct HoldState {
    locals: Vec<LocalHolds>,
}

/// The may-hold state before each statement and after each block.
pub(crate) struct Holds {
    /// `before[block][statement]` is the state before that statement.
    pub(crate) before: Vec<Vec<HoldState>>,
    /// `exit[block]` is the state after the block's last statement.
    pub(crate) exit: Vec<HoldState>,
}

impl HoldState {
    fn new(local_count: usize) -> Self {
        Self {
            locals: vec![LocalHolds::default(); local_count],
        }
    }

    /// Returns every loan any path of `local` may carry.
    pub(crate) fn local_loans(&self, local: LocalId) -> LoanSet {
        let mut set = LoanSet::default();
        for (_, loans) in &self.locals[local.index() as usize].paths {
            set.extend(loans.iter().copied());
        }
        set
    }

    /// Returns the loans a place may carry.
    ///
    /// Paths overlap when one is a prefix of the other: reading a whole
    /// aggregate includes references in its fields, and reading a field of a
    /// wholesale-assigned aggregate conservatively includes the aggregate's set.
    fn at(&self, place: &Place) -> LoanSet {
        let mut set = LoanSet::default();
        for (path, loans) in &self.locals[place.local.index() as usize].paths {
            if overlaps(path, &place.projections) {
                set.extend(loans.iter().copied());
            }
        }
        set
    }

    /// Replaces the loans carried by a place, dropping overwritten descendants.
    ///
    /// An ancestor entry is kept: it conservatively covers the whole storage,
    /// and dropping it could let a live loan go unnoticed.
    fn assign(&mut self, place: &Place, loans: LoanSet) {
        if place.is_deref() {
            return;
        }
        let paths = &mut self.locals[place.local.index() as usize].paths;
        paths.retain(|(path, _)| !path.starts_with(&place.projections));
        if !loans.is_empty() {
            paths.push((place.projections.clone(), loans));
        }
    }

    /// Drops the loans carried by a place and its descendants.
    fn clear(&mut self, place: &Place) {
        if place.is_deref() {
            return;
        }
        self.locals[place.local.index() as usize]
            .paths
            .retain(|(path, _)| !path.starts_with(&place.projections));
    }

    /// Unions `other` into this state.
    fn join(&mut self, other: &Self) -> bool {
        let mut changed = false;
        for (local, other_local) in other.locals.iter().enumerate() {
            let paths = &mut self.locals[local].paths;
            for (path, loans) in &other_local.paths {
                match paths.iter_mut().find(|(existing, _)| existing == path) {
                    Some((_, existing)) => {
                        for loan in loans {
                            changed |= existing.insert(*loan);
                        }
                    }
                    None => {
                        paths.push((path.clone(), loans.clone()));
                        changed = true;
                    }
                }
            }
        }
        changed
    }
}

/// Returns `true` when one path is a prefix of the other.
fn overlaps(left: &[Projection], right: &[Projection]) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

/// Computes the may-hold sets.
///
/// A loaded reference (reading `*p` where the pointee is itself a reference)
/// may alias any loan in the body, because the checker does not model the
/// pointee's contents. A call returning a reference is assumed to return a
/// reference derived from its arguments.
pub(crate) fn analyze(
    hir: &Hir,
    body: &Body,
    loans: &[Loan],
    loan_at: &[Vec<Option<LoanId>>],
) -> Holds {
    let local_count = body.locals.len();
    let all_loans: LoanSet = (0..loans.len()).map(LoanId::from_usize).collect();

    let mut in_states: Vec<Option<HoldState>> = vec![None; body.blocks.len()];
    in_states[body.entry.index() as usize] = Some(HoldState::new(local_count));
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
            .unwrap_or_else(|| HoldState::new(local_count));
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
    let destination_has_reference = hir.ty_ctxt().contains_reference(body.local(place.local).ty);

    match rvalue {
        Rvalue::Ref { .. } => {
            let mut loans = LoanSet::default();
            if let Some(loan) = loan {
                loans.insert(loan);
            }
            state.assign(place, loans);
        }
        Rvalue::Use(operand) => {
            let loans = match operand.place() {
                Some(source) if source.is_deref() => all_loans.clone(),
                Some(source) => state.at(&source),
                None => LoanSet::default(),
            };
            if let Some(source) = operand.place()
                && matches!(operand, Operand::Move(_))
            {
                state.clear(&source);
            }
            if destination_has_reference {
                state.assign(place, loans);
            } else {
                state.clear(place);
            }
        }
        Rvalue::Call { args, .. } => {
            let mut loans = LoanSet::default();
            if destination_has_reference {
                for arg in args {
                    match arg.place() {
                        Some(source) if source.is_deref() => {
                            loans.extend(all_loans.iter().copied())
                        }
                        Some(source) => loans.extend(state.at(&source)),
                        None => {}
                    }
                }
                state.assign(place, loans);
            } else {
                state.clear(place);
            }
        }
        Rvalue::Aggregate { fields, .. } => {
            // Record loans per field, not for the whole aggregate: a later write
            // to one field must drop only that field's loans.
            state.clear(place);
            if destination_has_reference {
                for (field, operand) in fields {
                    let loans = match operand.place() {
                        Some(source) if source.is_deref() => all_loans.clone(),
                        Some(source) => state.at(&source),
                        None => LoanSet::default(),
                    };
                    if !loans.is_empty() {
                        state.assign(&place.clone().field(*field), loans);
                    }
                }
            }
        }
        Rvalue::Binary { .. } | Rvalue::Unary { .. } => state.clear(place),
    }
}

/// Returns the loans held in the return place, for escape checks.
pub(crate) fn return_loans(holds: &HoldState, return_local: LocalId) -> Vec<LoanId> {
    let mut seen: FxHashSet<LoanId> = FxHashSet::default();
    let mut result = Vec::new();
    for (_, loans) in &holds.locals[return_local.index() as usize].paths {
        for loan in loans {
            if seen.insert(*loan) {
                result.push(*loan);
            }
        }
    }
    result
}
