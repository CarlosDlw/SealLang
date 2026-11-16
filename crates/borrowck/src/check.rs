//! The checking walk: conflicts, initialization, and escaping references.

use seal_diagnostics::DiagCtxt;
use seal_hir::Hir;
use seal_mir::{
    BasicBlock, Body, LocalId, Operand, Place, Rvalue, Statement, StatementKind, TerminatorKind,
};
use seal_symbol::Interner;
use seal_ty::InitPaths;

use crate::holds::{self, HoldState, LoanSet};
use crate::init::{self, Init, InitInfo};
use crate::liveness::{self, LocalSet};
use crate::loan::Loan;
use crate::loans;
use crate::report::Reporter;

/// Facts shared by every check in one body at one program point.
struct Context<'a> {
    loans: &'a [Loan],
    init: &'a InitPaths<LocalId>,
    active: &'a LoanSet,
    holds: &'a holds::Holds,
    hir: &'a Hir,
    body: &'a Body,
    reporter: &'a Reporter<'a>,
}

/// Checks one body.
pub(crate) fn check_body(hir: &Hir, body: &Body, dcx: &DiagCtxt, interner: &Interner) {
    let (loans, loan_at) = loans::collect(body);
    let liveness = liveness::analyze(body);
    let holds = holds::analyze(hir, body, &loans, &loan_at);
    let init = init::analyze(body);
    let reporter = Reporter::new(hir, dcx, body, interner);

    for (block_id, block) in body.blocks.iter_enumerated() {
        let index = block_id.index() as usize;
        for (position, statement) in block.statements.iter().enumerate() {
            let active = active_loans(
                &liveness.before[index][position],
                &holds.before[index][position],
            );
            let context = Context {
                loans: &loans,
                init: &init.before[index][position],
                active: &active,
                holds: &holds,
                hir,
                body,
                reporter: &reporter,
            };
            check_statement(statement, &context);
        }
        let context = Context {
            loans: &loans,
            init: &init.exit[index],
            active: &LoanSet::default(),
            holds: &holds,
            hir,
            body,
            reporter: &reporter,
        };
        check_terminator(index, block, &init, &context);
    }
}

/// Returns the loans carried by locals that are live at this point.
fn active_loans(live: &LocalSet, holds: &HoldState) -> LoanSet {
    let mut active = LoanSet::default();
    for local in live {
        active.extend(holds.local_loans(*local));
    }
    active
}

/// Checks one statement: reads first, then the write to its destination.
fn check_statement(statement: &Statement, context: &Context<'_>) {
    let StatementKind::Assign { place, rvalue } = &statement.kind;

    match rvalue {
        Rvalue::Use(operand) => check_operand(statement, operand, context),
        Rvalue::Ref(borrowed) => check_borrow(borrowed, statement.span, context),
        Rvalue::Binary { lhs, rhs, .. } => {
            check_operand(statement, lhs, context);
            check_operand(statement, rhs, context);
        }
        Rvalue::Unary { operand, .. } => check_operand(statement, operand, context),
        Rvalue::Call { args, .. } => {
            for arg in args {
                check_operand(statement, arg, context);
            }
        }
        Rvalue::Aggregate { fields, .. } => {
            for (_, operand) in fields {
                check_operand(statement, operand, context);
            }
        }
    }

    if place.is_deref() {
        return;
    }
    for loan in conflicting(context.active, context.loans, place) {
        context
            .reporter
            .assign_while_borrowed(place, statement.span, loan);
    }
}

/// Checks an operand's read and, when it moves, its conflict with live loans.
fn check_operand(statement: &Statement, operand: &Operand, context: &Context<'_>) {
    let Some(read) = operand.place() else {
        return;
    };
    check_read(&read, statement.span, context);

    if matches!(operand, Operand::Move(_)) && !read.is_deref() {
        for loan in conflicting(context.active, context.loans, &read) {
            context
                .reporter
                .move_while_borrowed(&read, statement.span, loan);
        }
    }
}

/// Requires a place to be initialized before it is read, borrowed, or moved.
fn check_read(place: &Place, span: seal_span::Span, context: &Context<'_>) {
    match state(context, place) {
        Init::Initialized => {}
        Init::Moved | Init::PartiallyMoved => context.reporter.use_of_moved_value(place, span),
        Init::Uninitialized => context.reporter.use_of_uninitialized(place, span),
    }
}

/// Checks a borrow: the place must be initialized and not partially moved.
fn check_borrow(place: &Place, span: seal_span::Span, context: &Context<'_>) {
    match state(context, place) {
        Init::Initialized => {}
        Init::Moved => context.reporter.use_of_moved_value(place, span),
        Init::PartiallyMoved => context.reporter.borrow_partially_moved(place, span),
        Init::Uninitialized => context.reporter.use_of_uninitialized(place, span),
    }
}

fn state(context: &Context<'_>, place: &Place) -> Init {
    init::state_of(
        context.init,
        place,
        context.hir.ty_ctxt(),
        context.body.local(place.local).ty,
    )
}

/// Returns the live loans that overlap `place`.
///
/// Two places overlap when one is a prefix of the other: borrowing `p.x`
/// prevents writing `p`, and borrowing `p` prevents writing `p.x`. Loans
/// through a reference are not compared, because the checker does not track
/// aliasing into borrowed data.
fn conflicting<'a>(active: &'a LoanSet, loans: &'a [Loan], place: &Place) -> Vec<&'a Loan> {
    if place.is_deref() {
        return Vec::new();
    }
    active
        .iter()
        .filter_map(|id| {
            let loan = &loans[id.index() as usize];
            (!loan.place.is_deref() && places_overlap(&loan.place, place)).then_some(loan)
        })
        .collect()
}

fn places_overlap(left: &Place, right: &Place) -> bool {
    left.local == right.local && (left.is_prefix_of(right) || right.is_prefix_of(left))
}

/// Checks a terminator: its reads, the return value, and reference escape.
fn check_terminator(
    block_index: usize,
    block: &BasicBlock,
    init: &InitInfo,
    context: &Context<'_>,
) {
    match &block.terminator.kind {
        TerminatorKind::SwitchInt { discriminant, .. } => {
            if let Some(place) = discriminant.place() {
                let exit = &init.exit[block_index];
                let context = Context {
                    init: exit,
                    ..*context
                };
                check_read(&place, block.terminator.span, &context);
            }
        }
        TerminatorKind::Return => {
            let exit = &init.exit[block_index];
            let return_local = Body::return_local();
            let read_context = Context {
                init: exit,
                ..*context
            };
            check_read(
                &Place::local(return_local),
                block.terminator.span,
                &read_context,
            );
            for id in holds::return_loans(&context.holds.exit[block_index], return_local) {
                let loan = &context.loans[id.index() as usize];
                // A loan through a reference borrows data outside this frame;
                // a loan of this frame's storage would dangle on return.
                if loan.place.is_deref() {
                    continue;
                }
                context
                    .reporter
                    .return_local_reference(loan, block.terminator.span);
            }
        }
        TerminatorKind::Goto(_) | TerminatorKind::Unreachable => {}
    }
}
