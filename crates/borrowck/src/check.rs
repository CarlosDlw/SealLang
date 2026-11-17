//! The checking walk: conflicts, initialization, and escaping references.

use seal_diagnostics::DiagCtxt;
use seal_hir::Hir;
use seal_mir::{
    BasicBlock, Body, LocalId, Operand, Place, Rvalue, Statement, StatementKind, TerminatorKind,
};
use seal_symbol::Interner;
use seal_ty::{InitPaths, RefKind, TyKind};

use crate::aliasing::{BodyFacts, CallSites};
use crate::calls;
use crate::holds::{self, HoldState, LoanSet};
use crate::id::LoanId;
use crate::init::{self, Init, InitInfo};
use crate::liveness::{self, LocalSet};
use crate::loan::Loan;
use crate::loans;
use crate::origins::{self, OriginState};
use crate::report::Reporter;

/// Facts shared by every check in one body at one program point.
struct Context<'a> {
    loans: &'a [Loan],
    init: &'a InitPaths<LocalId>,
    active: &'a LoanSet,
    holds: &'a holds::Holds,
    holds_at: &'a HoldState,
    origins: &'a OriginState,
    /// The loan this statement creates, when it is a borrow.
    current_loan: Option<LoanId>,
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

/// Checks one body and records the alias facts its parameters depend on.
pub(crate) fn check_body(
    hir: &Hir,
    body: &Body,
    dcx: &DiagCtxt,
    interner: &Interner,
    calls: &mut CallSites,
) -> BodyFacts {
    let (mut loans, loan_at) = loans::collect(hir, body);
    let liveness = liveness::analyze(body);
    let holds = holds::analyze(hir, body, &loans, &loan_at);
    let origins = origins::analyze(hir, body);
    loans::resolve(body, &mut loans, &loan_at, &origins, &holds);
    record_last_uses(body, &liveness, &holds, &mut loans);
    let init = init::analyze(body);
    let reporter = Reporter::new(hir, dcx, body, interner);

    let mut unknown = false;
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
                holds_at: &holds.before[index][position],
                origins: &origins.before[index][position],
                current_loan: loan_at[index][position],
                hir,
                body,
                reporter: &reporter,
            };
            unknown |= check_statement(statement, &context, calls);
        }
        // A terminator still reads locals, so the loans those locals hold are
        // live there too: `if *r == 0` must see the borrow that `r` carries.
        let active = active_loans(&liveness.after[index], &holds.exit[index]);
        let context = Context {
            loans: &loans,
            init: &init.exit[index],
            active: &active,
            holds: &holds,
            holds_at: &holds.exit[index],
            origins: &origins.exit[index],
            current_loan: None,
            hir,
            body,
            reporter: &reporter,
        };
        unknown |= check_terminator(index, block, &init, &context);
    }
    BodyFacts { unknown }
}

/// Returns the loans carried by locals that are live at this point.
fn active_loans(live: &LocalSet, holds: &HoldState) -> LoanSet {
    let mut active = LoanSet::default();
    for local in live {
        active.extend(holds.local_loans(*local));
    }
    active
}

/// Records where each loan is last active.
///
/// A borrow ends at its last use, so that point is where the user has to stop
/// relying on the reference before a conflicting access becomes valid. A loan
/// no live local ever carries (an unused borrow) has no last use.
fn record_last_uses(
    body: &Body,
    liveness: &liveness::Liveness,
    holds: &holds::Holds,
    loans: &mut [Loan],
) {
    for (index, block) in body.blocks.iter().enumerate() {
        for (position, statement) in block.statements.iter().enumerate() {
            let active = active_loans(
                &liveness.before[index][position],
                &holds.before[index][position],
            );
            for id in active {
                loans[id.index() as usize].last_use = Some(statement.span);
            }
        }
        let active = active_loans(&liveness.after[index], &holds.exit[index]);
        for id in active {
            loans[id.index() as usize].last_use = Some(block.terminator.span);
        }
    }
}

/// Checks one statement: reads first, then the write to its destination.
///
/// Returns whether the statement reached storage through a reference whose
/// target the may-name analysis does not model.
fn check_statement(statement: &Statement, context: &Context<'_>, calls: &mut CallSites) -> bool {
    let StatementKind::Assign { place, rvalue } = &statement.kind;
    let mut unknown = origins::keys_of(context.origins, place).is_unknown();

    match rvalue {
        Rvalue::Use(operand) => {
            unknown |= operand_unknown(operand, context);
            check_operand(statement, operand, context);
        }
        Rvalue::Discriminant(place) => {
            unknown |= origins::keys_of(context.origins, place).is_unknown();
            check_read(place, statement.span, context);
        }
        Rvalue::Ref { place, kind } => {
            unknown |= origins::keys_of(context.origins, place).is_unknown();
            check_borrow(place, *kind, statement.span, context);
        }
        Rvalue::SliceRef {
            base,
            start,
            end,
            kind,
            ..
        } => {
            let borrow_place = if matches!(
                context
                    .hir
                    .ty_ctxt()
                    .kind(origins::place_ty(context.hir, context.body, base)),
                TyKind::Ref { .. }
            ) {
                check_read(base, statement.span, context);
                base.clone().deref()
            } else {
                base.clone()
            };
            unknown |= origins::keys_of(context.origins, &borrow_place).is_unknown();
            check_borrow(&borrow_place, *kind, statement.span, context);
            for operand in start.iter().chain(end.iter()) {
                unknown |= operand_unknown(operand, context);
                check_operand(statement, operand, context);
            }
        }
        Rvalue::Len { slice } => {
            unknown |= operand_unknown(slice, context);
            check_operand(statement, slice, context);
        }
        Rvalue::Tuple { elements, .. } => {
            for element in elements {
                unknown |= operand_unknown(element, context);
                check_operand(statement, element, context);
            }
        }
        Rvalue::Binary { lhs, rhs, .. } => {
            unknown |= operand_unknown(lhs, context) || operand_unknown(rhs, context);
            check_operand(statement, lhs, context);
            check_operand(statement, rhs, context);
        }
        Rvalue::Unary { operand, .. } => {
            unknown |= operand_unknown(operand, context);
            check_operand(statement, operand, context);
        }
        Rvalue::Intrinsic {
            method,
            receiver,
            args,
            ..
        } => {
            let exclusive = method.def().receiver == seal_methods::ReceiverMode::Exclusive;
            match receiver {
                seal_mir::IntrinsicReceiver::Place(place) => {
                    unknown |= origins::keys_of(context.origins, place).is_unknown();
                    if exclusive {
                        check_borrow(place, RefKind::Exclusive, statement.span, context);
                    } else {
                        check_read(place, statement.span, context);
                    }
                }
                seal_mir::IntrinsicReceiver::Value(operand) => {
                    unknown |= operand_unknown(operand, context);
                    check_operand(statement, operand, context);
                }
            }
            for arg in args {
                unknown |= operand_unknown(arg, context);
                check_operand(statement, arg, context);
            }
        }
        Rvalue::Call {
            target,
            args,
            arg_spans,
        } => {
            for arg in args {
                check_operand(statement, arg, context);
            }
            let outcome = calls::check(calls::CallSite {
                args,
                arg_spans,
                span: statement.span,
                origins: context.origins,
                holds: context.holds_at,
                loans: context.loans,
                active: context.active,
                hir: context.hir,
                body: context.body,
                reporter: context.reporter,
            });
            unknown |= outcome.unknown;
            // A symbolic trait call has no concrete callee whose `noalias`
            // contract could be proven here; monomorphization checks each
            // selected implementation at its own call sites.
            if let seal_mir::CallTarget::Direct { instance, .. } = target {
                calls.record(instance.def, outcome.proven);
            }
        }
        Rvalue::Aggregate { fields, .. } => {
            for (_, operand) in fields {
                unknown |= operand_unknown(operand, context);
                check_operand(statement, operand, context);
            }
        }
        Rvalue::Array { elements, .. } => {
            for element in elements {
                unknown |= operand_unknown(element, context);
                check_operand(statement, element, context);
            }
        }
        Rvalue::Repeat { value, .. } => {
            unknown |= operand_unknown(value, context);
            check_operand(statement, value, context);
        }
    }

    if !place_is_reported(context, place) {
        for id in overlapping(context, place) {
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
    unknown
}

/// Returns whether reading `operand` reaches storage through an unknown target.
fn operand_unknown(operand: &Operand, context: &Context<'_>) -> bool {
    operand
        .place()
        .is_some_and(|place| origins::keys_of(context.origins, &place).is_unknown())
}

/// Checks an operand's read and, when it moves, its conflict with live loans.
fn check_operand(statement: &Statement, operand: &Operand, context: &Context<'_>) {
    let Some(read) = operand.place() else {
        return;
    };
    check_read(&read, statement.span, context);

    if matches!(operand, Operand::Move(_)) && !read.is_deref() && !place_is_reported(context, &read)
    {
        for id in overlapping(context, &read) {
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
    if !place_is_reported(context, place) {
        for id in overlapping(context, place) {
            let loan = context.loan(id);
            if loan.kind.is_exclusive() {
                context
                    .reporter
                    .read_while_exclusively_borrowed(place, span, loan);
            }
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
    // A borrow through a reference that was already rejected carries the same
    // mistake forward; mark it rejected too and stay silent.
    let cascade = place_is_reported(context, place);
    let mut conflicted = false;
    if !cascade {
        for id in overlapping(context, place) {
            let loan = context.loan(id);
            if kind.is_exclusive() || loan.kind.is_exclusive() {
                context.reporter.borrow_conflict(place, kind, span, loan);
                conflicted = true;
            }
        }
    }
    if let Some(id) = context.current_loan
        && (cascade || conflicted)
    {
        context.loans[id.index() as usize].reported.set(true);
    }
}

/// Returns whether an access reaches storage through a rejected borrow.
///
/// A rejected borrow (`E5008`) leaves the reference usable in the MIR, but a
/// later access through it would only repeat the cause the user already saw, so
/// conflict diagnostics on that access are suppressed.
fn place_is_reported(context: &Context<'_>, place: &Place) -> bool {
    origins::first_deref_base(place).is_some_and(|base| {
        context
            .holds_at
            .at(&base)
            .iter()
            .any(|id| context.loan(*id).reported.get())
    })
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

/// Returns the live loans whose storage overlaps `place`, in loan order.
///
/// Storage overlap is decided by the may-name analysis, not by place syntax, so
/// `*p` conflicts with a loan of the storage `p` may point at. The loans that
/// authorize the access are excluded: a reference may use its own loan, and a
/// reborrow may use the loans it was derived from. The IDs are sorted so
/// diagnostics do not depend on hash iteration order.
fn overlapping(context: &Context<'_>, place: &Place) -> Vec<LoanId> {
    let keys = origins::keys_of(context.origins, place);
    if keys.is_empty() {
        return Vec::new();
    }
    let mut excluded = LoanSet::default();
    if place.is_deref() {
        if let Some(base) = origins::first_deref_base(place) {
            for id in context.holds_at.at(&base) {
                if origins::keys_overlap(&keys, &context.loan(id).keys) {
                    excluded.insert(id);
                }
            }
        }
        crate::loan::expand_parents(context.loans, &mut excluded);
    }
    let mut ids: Vec<LoanId> = context
        .active
        .iter()
        .copied()
        .filter(|id| {
            !excluded.contains(id)
                && !context.loan(*id).reported.get()
                && origins::keys_overlap(&keys, &context.loan(*id).keys)
        })
        .collect();
    ids.sort_by_key(|id| id.index());
    ids
}

/// Checks a terminator: its reads, the return value, and reference escape.
///
/// Returns whether the terminator reached storage through an unknown target.
fn check_terminator(
    block_index: usize,
    block: &BasicBlock,
    init: &InitInfo,
    context: &Context<'_>,
) -> bool {
    match &block.terminator.kind {
        TerminatorKind::SwitchInt { discriminant, .. } => {
            let mut unknown = false;
            if let Some(place) = discriminant.place() {
                unknown = origins::keys_of(context.origins, &place).is_unknown();
                let exit = &init.exit[block_index];
                let context = Context {
                    init: exit,
                    ..*context
                };
                check_read(&place, block.terminator.span, &context);
            }
            unknown
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
                if loan.reported.get() {
                    continue;
                }
                context
                    .reporter
                    .return_local_reference(loan, block.terminator.span);
            }
            false
        }
        TerminatorKind::Goto(_) | TerminatorKind::Panic { .. } | TerminatorKind::Unreachable => {
            false
        }
    }
}
