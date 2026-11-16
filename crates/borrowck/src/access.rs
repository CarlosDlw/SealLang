//! Reading the uses and definitions out of statements.

use seal_hash::FxHashSet;
use seal_mir::{Body, LocalId, Operand, Rvalue, Statement, StatementKind, TerminatorKind};

/// A set of locals.
pub(crate) type LocalSet = FxHashSet<LocalId>;

/// Adds every local a statement reads.
///
/// Writing through a dereference also reads the reference, and borrowing a
/// place reads it: both need the base local to be initialized even though no
/// value is moved. Writing a field does not read the base local as a value.
pub(crate) fn statement_uses(statement: &Statement, uses: &mut LocalSet) {
    let StatementKind::Assign { place, rvalue } = &statement.kind;
    if place.is_deref() {
        uses.insert(place.local);
    }
    rvalue_uses(rvalue, uses);
}

/// Returns the local a statement assigns, when it defines the whole local.
///
/// A field write leaves the other fields and the local itself live, so it is
/// not a definition for liveness.
pub(crate) fn statement_def(statement: &Statement) -> Option<LocalId> {
    let StatementKind::Assign { place, .. } = &statement.kind;
    (!place.is_deref() && place.is_whole()).then_some(place.local)
}

/// Adds the locals a value expression reads.
pub(crate) fn rvalue_uses(rvalue: &Rvalue, uses: &mut LocalSet) {
    match rvalue {
        Rvalue::Use(operand) => operand_uses(operand, uses),
        Rvalue::Ref(place) => {
            uses.insert(place.local);
        }
        Rvalue::Binary { lhs, rhs, .. } => {
            operand_uses(lhs, uses);
            operand_uses(rhs, uses);
        }
        Rvalue::Unary { operand, .. } => operand_uses(operand, uses),
        Rvalue::Call { args, .. } => {
            for arg in args {
                operand_uses(arg, uses);
            }
        }
        Rvalue::Aggregate { fields, .. } => {
            for (_, operand) in fields {
                operand_uses(operand, uses);
            }
        }
    }
}

/// Adds the local an operand reads.
pub(crate) fn operand_uses(operand: &Operand, uses: &mut LocalSet) {
    if let Some(place) = operand.place() {
        uses.insert(place.local);
    }
}

/// Adds the locals a terminator reads.
pub(crate) fn terminator_uses(kind: &TerminatorKind, uses: &mut LocalSet) {
    match kind {
        TerminatorKind::SwitchInt { discriminant, .. } => operand_uses(discriminant, uses),
        TerminatorKind::Return => {
            uses.insert(Body::return_local());
        }
        TerminatorKind::Goto(_) | TerminatorKind::Unreachable => {}
    }
}
