//! The checking walk: conflicts, initialization, and escaping references.

use seal_diagnostics::DiagCtxt;
use seal_hash::FxHashSet;
use seal_hir::Hir;
use seal_mir::{
    BasicBlock, Body, LocalId, Operand, Place, Rvalue, Statement, StatementKind, TerminatorKind,
};
use seal_symbol::Interner;

use crate::holds::{self, HoldState, LoanSet};
use crate::init::{self, Init, InitState};
use crate::liveness::{self, LocalSet};
use crate::loan::Loan;
use crate::loans;
use crate::report::Reporter;

/// Checks one body.
pub(crate) fn check_body(hir: &Hir, body: &Body, dcx: &DiagCtxt, interner: &Interner) {
    let (loans, loan_at) = loans::collect(body);
    let liveness = liveness::analyze(body);
    let holds = holds::analyze(hir, body, &loans, &loan_at);
    let init = init::analyze(body);
    let reporter = Reporter::new(dcx, body, interner);

    for (block_id, block) in body.blocks.iter_enumerated() {
        let index = block_id.index() as usize;
        for (position, statement) in block.statements.iter().enumerate() {
            let active = active_loans(
                &liveness.before[index][position],
                &holds.before[index][position],
            );
            check_statement(
                statement,
                &loans,
                &init.before[index][position],
                &active,
                &reporter,
            );
        }
        check_terminator(
            block,
            &loans,
            &init.exit[index],
            &holds.exit[index],
            &reporter,
        );
    }
}

/// Returns the loans carried by locals that are live at this point.
fn active_loans(live: &LocalSet, holds: &HoldState) -> LoanSet {
    let mut active = LoanSet::default();
    for local in live {
        active.extend(holds[local.index() as usize].iter().copied());
    }
    active
}

/// Checks one statement: reads first, then the write to its destination.
fn check_statement(
    statement: &Statement,
    loans: &[Loan],
    init: &InitState,
    active: &LoanSet,
    reporter: &Reporter<'_>,
) {
    let StatementKind::Assign { place, rvalue } = &statement.kind;

    match rvalue {
        Rvalue::Use(operand) => check_operand(statement, *operand, loans, init, active, reporter),
        Rvalue::Ref(borrowed) => check_read(*borrowed, statement.span, init, reporter),
        Rvalue::Binary { lhs, rhs, .. } => {
            check_operand(statement, *lhs, loans, init, active, reporter);
            check_operand(statement, *rhs, loans, init, active, reporter);
        }
        Rvalue::Unary { operand, .. } => {
            check_operand(statement, *operand, loans, init, active, reporter);
        }
        Rvalue::Call { args, .. } => {
            for arg in args {
                check_operand(statement, *arg, loans, init, active, reporter);
            }
        }
    }

    if !place.is_deref() {
        for loan in conflicting(active, loans, *place) {
            reporter.assign_while_borrowed(place.local, statement.span, loan);
        }
    }
}

/// Checks an operand's read and, when it moves, its conflict with live loans.
fn check_operand(
    statement: &Statement,
    operand: Operand,
    loans: &[Loan],
    init: &InitState,
    active: &LoanSet,
    reporter: &Reporter<'_>,
) {
    let Some(read) = operand.place() else {
        return;
    };
    check_read(read, statement.span, init, reporter);

    if matches!(operand, Operand::Move(_)) && !read.is_deref() {
        for loan in conflicting(active, loans, read) {
            reporter.move_while_borrowed(read.local, statement.span, loan);
        }
    }
}

/// Requires a place to be initialized before it is read, borrowed, or moved.
fn check_read(place: Place, span: seal_span::Span, init: &InitState, reporter: &Reporter<'_>) {
    match init::state_of(init, place.local) {
        Init::Initialized => {}
        Init::Moved => reporter.use_of_moved_value(place.local, span),
        Init::Uninitialized => reporter.use_of_uninitialized(place.local, span),
    }
}

/// Returns the live loans that borrow exactly `place`.
fn conflicting<'a>(active: &'a LoanSet, loans: &'a [Loan], place: Place) -> Vec<&'a Loan> {
    active
        .iter()
        .filter_map(|id| {
            let loan = &loans[id.index() as usize];
            (loan.place == place).then_some(loan)
        })
        .collect()
}

/// Checks a terminator: its reads, the return value, and reference escape.
fn check_terminator(
    block: &BasicBlock,
    loans: &[Loan],
    init: &InitState,
    holds: &HoldState,
    reporter: &Reporter<'_>,
) {
    match &block.terminator.kind {
        TerminatorKind::SwitchInt { discriminant, .. } => {
            if let Some(place) = discriminant.place() {
                check_read(place, block.terminator.span, init, reporter);
            }
        }
        TerminatorKind::Return => {
            let return_local = Body::return_local();
            check_read(
                Place::local(return_local),
                block.terminator.span,
                init,
                reporter,
            );
            let mut reported: FxHashSet<LocalId> = FxHashSet::default();
            for id in &holds[return_local.index() as usize] {
                let loan = &loans[id.index() as usize];
                // A loan through a reference borrows data outside this frame;
                // a loan of a whole local names this frame's storage and would
                // dangle once the function returns.
                if loan.place.is_deref() {
                    continue;
                }
                if reported.insert(loan.place.local) {
                    reporter.return_local_reference(loan, block.terminator.span);
                }
            }
        }
        TerminatorKind::Goto(_) | TerminatorKind::Unreachable => {}
    }
}
