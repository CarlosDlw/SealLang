//! The checking walk: conflicts, initialization, and escaping references.

use seal_diagnostics::DiagCtxt;
use seal_hir::Hir;
use seal_mir::{
    BasicBlock, Body, LocalId, Operand, Place, Rvalue, Statement, StatementKind, TerminatorKind,
};
use seal_symbol::Interner;
use seal_ty::{InitPaths, RefKind};

use crate::holds::{self, HoldState, LoanSet};
use crate::id::LoanId;
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

impl Context<'_> {
    /// Returns the loan with `id`; every active ID comes from the loan table.
    fn loan(&self, id: LoanId) -> &Loan {
        &self.loans[id.index() as usize]
    }
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
        // A terminator still reads locals, so the loans those locals hold are
        // live there too: `if *r == 0` must see the borrow that `r` carries.
        let active = active_loans(&liveness.after[index], &holds.exit[index]);
        let context = Context {
            loans: &loans,
            init: &init.exit[index],
            active: &active,
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
        Rvalue::Ref { place, kind } => check_borrow(place, *kind, statement.span, context),
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
    for id in overlapping(context.active, context.loans, place) {
        let loan = context.loan(id);
        if loan.kind.is_exclusive() {
            context
                .reporter
                .write_while_exclusively_borrowed(place, statement.span, loan);
        } else {
            context
                .reporter
                .assign_while_borrowed(place, statement.span, loan);
        }
    }
}

/// Checks an operand's read and, when it moves, its conflict with live loans.
fn check_operand(statement: &Statement, operand: &Operand, context: &Context<'_>) {
    let Some(read) = operand.place() else {
        return;
    };
    check_read(&read, statement.span, context);

    if matches!(operand, Operand::Move(_)) && !read.is_deref() {
        for id in overlapping(context.active, context.loans, &read) {
            let loan = context.loan(id);
            if loan.kind.is_exclusive() {
                context
                    .reporter
                    .move_while_exclusively_borrowed(&read, statement.span, loan);
            } else {
                context
                    .reporter
                    .move_while_borrowed(&read, statement.span, loan);
            }
        }
    }
}

/// Requires a place to be initialized before it is read, borrowed, or moved.
fn check_read(place: &Place, span: seal_span::Span, context: &Context<'_>) {
    match state(context, place) {
        Init::Initialized => {}
        Init::Moved | Init::PartiallyMoved => {
            let moved = moved_place(context, place);
            context.reporter.use_of_moved_value(place, &moved, span);
        }
        Init::Uninitialized => context.reporter.use_of_uninitialized(place, span),
    }
    for id in overlapping(context.active, context.loans, place) {
        let loan = context.loan(id);
        if loan.kind.is_exclusive() {
            context
                .reporter
                .read_while_exclusively_borrowed(place, span, loan);
        }
    }
}

/// Checks a borrow: the place must be initialized and not partially moved, and
/// the new loan must not overlap a live one with an incompatible kind.
fn check_borrow(place: &Place, kind: RefKind, span: seal_span::Span, context: &Context<'_>) {
    match state(context, place) {
        Init::Initialized => {}
        Init::Moved => {
            let moved = moved_place(context, place);
            context.reporter.use_of_moved_value(place, &moved, span);
        }
        Init::PartiallyMoved => context.reporter.borrow_partially_moved(place, span),
        Init::Uninitialized => context.reporter.use_of_uninitialized(place, span),
    }
    for id in overlapping(context.active, context.loans, place) {
        let loan = context.loan(id);
        if kind.is_exclusive() || loan.kind.is_exclusive() {
            context.reporter.borrow_conflict(place, kind, span, loan);
        }
    }
}

/// Returns the place whose value was moved out, which may be a prefix of
/// `place`.
///
/// A read of `a.x` after `a` moved is fixed by reassigning `a`, so the message
/// names the shortest prefix that is itself moved instead of reporting the
/// read as if `a.x` had been moved on its own.
fn moved_place(context: &Context<'_>, place: &Place) -> Place {
    (0..=place.projections.len())
        .map(|length| place.projections[..length].to_vec())
        .find_map(|projections| {
            let prefix = Place::with_projections(place.local, projections);
            matches!(state(context, &prefix), Init::Moved).then_some(prefix)
        })
        .unwrap_or_else(|| place.clone())
}

fn state(context: &Context<'_>, place: &Place) -> Init {
    init::state_of(
        context.init,
        place,
        context.hir.ty_ctxt(),
        context.body.local(place.local).ty,
    )
}

/// Collects the frame loans that escape with a returned loan.
///
/// A loan created by `&*reference` borrows whatever that reference points at. A
/// reference created in this frame therefore carries its own loans out with the
/// returned reference, and those are the loans that would dangle. A reference
/// loaded through another reference may alias any loan in the body, which is
/// the conservative reading the may-hold analysis already documents.
fn escaping_loans(
    holds: &HoldState,
    loans: &[Loan],
    id: LoanId,
    visited: &mut LoanSet,
    escaped: &mut Vec<LoanId>,
) {
    if !visited.insert(id) {
        return;
    }
    let loan = &loans[id.index() as usize];
    if !loan.place.is_deref() {
        escaped.push(id);
        return;
    }
    for held in holds.local_loans(loan.place.local) {
        escaping_loans(holds, loans, held, visited, escaped);
    }
}

/// Returns the live loans that overlap `place`, in loan order.
///
/// Two places overlap when one is a prefix of the other: borrowing `p.x`
/// prevents writing `p`, and borrowing `p` prevents writing `p.x`. Loans and
/// places through a reference are not compared, because the checker does not
/// track aliasing into borrowed data. The IDs are sorted so diagnostics do not
/// depend on hash iteration order.
fn overlapping(active: &LoanSet, loans: &[Loan], place: &Place) -> Vec<LoanId> {
    if place.is_deref() {
        return Vec::new();
    }
    let mut ids: Vec<LoanId> = active
        .iter()
        .copied()
        .filter(|id| {
            let loan = &loans[id.index() as usize];
            !loan.place.is_deref() && places_overlap(&loan.place, place)
        })
        .collect();
    ids.sort_by_key(|id| id.index());
    ids
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
            let mut escaped = Vec::new();
            let mut visited = LoanSet::default();
            for id in holds::return_loans(&context.holds.exit[block_index], return_local) {
                escaping_loans(
                    &context.holds.exit[block_index],
                    context.loans,
                    id,
                    &mut visited,
                    &mut escaped,
                );
            }
            // A loan through a reference borrows data outside this frame unless
            // the reference itself borrowed this frame's storage, so report the
            // loans that actually would dangle, in a deterministic order.
            escaped.sort_by_key(|id| id.index());
            for id in escaped {
                let loan = &context.loans[id.index() as usize];
                context
                    .reporter
                    .return_local_reference(loan, block.terminator.span);
            }
        }
        TerminatorKind::Goto(_) | TerminatorKind::Unreachable => {}
    }
}
