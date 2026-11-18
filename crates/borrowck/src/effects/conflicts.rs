use seal_hash::FxHashSet;
use seal_hir::Hir;
use seal_mir::Operand;

use crate::holds::LoanSet;
use crate::id::LoanId;
use crate::loan::Loan;
use crate::origins;

use super::{GlobalEffects, GlobalPath};

pub(crate) struct Conflict {
    /// The conflicting loan.
    pub(crate) loan: LoanId,
    /// The static path the callee touches.
    pub(crate) path: GlobalPath,
    /// Whether the access is a write.
    pub(crate) write: bool,
}

/// Applies a call's summary to the loans active at its site.
pub(crate) fn conflicts(
    hir: &Hir,
    effects: &GlobalEffects,
    args: &[Operand],
    origins: &origins::OriginState,
    loans: &[Loan],
    active: &LoanSet,
) -> Vec<Conflict> {
    let excluded = argument_paths(origins, args);
    let mut seen = FxHashSet::default();
    let mut conflicts = Vec::new();
    if effects.unknown {
        // A bodyless callee may touch any static; one conflict per static loan
        // is enough to reject the call.
        for id in active.iter().copied() {
            let loan = &loans[id.index() as usize];
            if let Some(path) = loan_paths(hir, loan).into_iter().next() {
                conflicts.push(Conflict {
                    loan: id,
                    path,
                    write: true,
                });
            }
        }
        return conflicts;
    }
    for path in &effects.reads {
        if excluded
            .iter()
            .any(|argument| argument_covers(argument, path))
        {
            continue;
        }
        for id in active.iter().copied() {
            let loan = &loans[id.index() as usize];
            if !loan.kind.is_exclusive()
                || !loan_paths(hir, loan).iter().any(|key| key.overlaps(path))
            {
                continue;
            }
            if seen.insert((id, false)) {
                conflicts.push(Conflict {
                    loan: id,
                    path: path.clone(),
                    write: false,
                });
            }
        }
    }
    for path in &effects.writes {
        if excluded
            .iter()
            .any(|argument| argument_covers(argument, path))
        {
            continue;
        }
        for id in active.iter().copied() {
            let loan = &loans[id.index() as usize];
            if !loan_paths(hir, loan).iter().any(|key| key.overlaps(path)) {
                continue;
            }
            if seen.insert((id, true)) {
                conflicts.push(Conflict {
                    loan: id,
                    path: path.clone(),
                    write: true,
                });
            }
        }
    }
    conflicts
}

fn loan_paths(hir: &Hir, loan: &Loan) -> Vec<GlobalPath> {
    // The borrowed place keeps constant indices, which storage keys erase to
    // keep runtime-indexed arrays conservative. Prefer it when it still names a
    // static mut; fall back to the resolved keys for reborrows through pointers.
    if let Some(path) = GlobalPath::from_place(hir, &loan.place) {
        return vec![path];
    }
    loan.keys
        .globals()
        .filter_map(|key| GlobalPath::from_storage(&key))
        .collect()
}

/// Returns whether a reference argument already accounts for an effect path.
///
/// The argument's grant covers only its own path and descendants. An effect on
/// a prefix (for example, writing the whole static through a field argument)
/// is a separate access and must still be checked.
fn argument_covers(argument: &GlobalPath, effect: &GlobalPath) -> bool {
    argument.def == effect.def && effect.steps.starts_with(&argument.steps)
}

fn argument_paths(origins: &origins::OriginState, args: &[Operand]) -> Vec<GlobalPath> {
    let mut paths = Vec::new();
    for arg in args {
        let Some(place) = arg.place() else {
            continue;
        };
        if place.is_deref() {
            continue;
        }
        for key in origins.at(&place).globals() {
            if let Some(path) = GlobalPath::from_storage(&key) {
                paths.push(path);
            }
        }
    }
    paths
}
