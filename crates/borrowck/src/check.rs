//! The checking walk: conflicts, initialization, and escaping references.

use seal_diagnostics::DiagCtxt;
use seal_hir::Hir;
use seal_mir::init::{self, Init, InitInfo};
use seal_mir::{
    BasicBlock, Body, DropKind, LocalId, Operand, Place, Rvalue, Statement, StatementKind,
    TerminatorKind,
};
use seal_span::Span;
use seal_symbol::Interner;
use seal_ty::{InitPaths, RefKind, TyKind};

use crate::aliasing::{BodyFacts, CallSites};
use crate::calls;
use crate::effects::{self, Effects};
use crate::holds::{self, HoldState, LoanSet};
use crate::id::LoanId;
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
    effects: &'a Effects,
    const_locals: &'a origins::ConstLocals,
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

    /// Returns the storage `place` may name.
    fn keys_of(&self, place: &Place) -> origins::OriginSet {
        origins::keys_of(self.origins, place, self.const_locals)
    }

    /// Applies a callee's mutable-static effects at a call site.
    fn apply_effects(
        &self,
        callee: seal_def::DefId,
        summary: effects::GlobalEffects,
        args: &[Operand],
        span: seal_span::Span,
    ) {
        for conflict in effects::conflicts(
            self.hir,
            &summary,
            args,
            self.origins,
            self.loans,
            self.active,
        ) {
            let loan = self.loan(conflict.loan);
            self.reporter
                .call_global_conflict(callee, &conflict.path, conflict.write, span, loan);
        }
    }
}

/// Checks one body and records the alias facts its parameters depend on.
pub(crate) fn check_body(
    hir: &Hir,
    body: &Body,
    program: Option<&seal_const_eval::ConstProgram>,
    effects: &Effects,
    dcx: &DiagCtxt,
    interner: &Interner,
    calls: &mut CallSites,
) -> BodyFacts {
    let (mut loans, loan_at) = loans::collect(hir, body);
    let liveness = liveness::analyze(body);
    let holds = holds::analyze(hir, body, &loans, &loan_at);
    let const_locals = origins::const_locals(body);
    let origins = origins::analyze(hir, body, program, &const_locals);
    loans::resolve(body, &mut loans, &loan_at, &origins, &holds, &const_locals);
    record_last_uses(body, &liveness, &holds, &mut loans);
    let init = init::analyze(hir, body);
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
                effects,
                const_locals: &const_locals,
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
            effects,
            const_locals: &const_locals,
            current_loan: None,
            hir,
            body,
            reporter: &reporter,
        };
        unknown |= check_terminator(index, block, &init, &context);
    }
    BodyFacts {
        unknown,
        touches_raw: effects.touches_raw(body.owner),
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
    let (place, rvalue) = match &statement.kind {
        StatementKind::Assign { place, rvalue } => (place, rvalue),
        StatementKind::Drop { place, kind } => {
            return check_drop(place, *kind, statement.span, context);
        }
        StatementKind::DropRemaining { place, from } => {
            return check_drop_remaining(place, from, statement, context);
        }
    };
    let mut unknown = context.keys_of(place).is_unknown();

    match rvalue {
        Rvalue::Use(operand) | Rvalue::Cast { operand, .. } => {
            unknown |= operand_unknown(operand, context);
            check_operand(statement, operand, context);
        }
        Rvalue::MakeTraitObject { source, .. } => {
            unknown |= operand_unknown(source, context);
            check_operand(statement, source, context);
        }
        Rvalue::TraitObjectReborrow {
            source, place, to, ..
        } => {
            unknown |= operand_unknown(source, context);
            check_operand(statement, source, context);
            unknown |= context.keys_of(place).is_unknown();
            check_borrow(
                place,
                loans::reborrow_kind(context.hir, *to),
                statement.span,
                context,
            );
        }
        Rvalue::DynamicCall {
            method,
            receiver,
            args,
            arg_spans,
            ..
        } => {
            check_operand(statement, receiver, context);
            check_shared_dynamic_receiver(receiver, statement.span, context);
            for arg in args {
                check_operand(statement, arg, context);
            }
            let mut all_args = Vec::with_capacity(args.len() + 1);
            all_args.push(receiver.clone());
            all_args.extend(args.iter().cloned());
            let outcome = calls::check(calls::CallSite {
                args: &all_args,
                arg_spans,
                span: statement.span,
                origins: context.origins,
                const_locals: context.const_locals,
                holds: context.holds_at,
                loans: context.loans,
                active: context.active,
                hir: context.hir,
                body: context.body,
                reporter: context.reporter,
            });
            unknown |= outcome.unknown;
            let summary = context.effects.dynamic(*method);
            context.apply_effects(*method, summary, &all_args, statement.span);
        }
        Rvalue::Discriminant(place) => {
            unknown |= context.keys_of(place).is_unknown();
            check_read(place, statement.span, context);
        }
        Rvalue::Ref { place, kind } => {
            unknown |= context.keys_of(place).is_unknown();
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
            unknown |= context.keys_of(&borrow_place).is_unknown();
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
                    unknown |= context.keys_of(place).is_unknown();
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
                const_locals: context.const_locals,
                holds: context.holds_at,
                loans: context.loans,
                active: context.active,
                hir: context.hir,
                body: context.body,
                reporter: context.reporter,
            });
            unknown |= outcome.unknown;
            let summary = context.effects.call(*target);
            context.apply_effects(target.instance().def, summary, args, statement.span);
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

    check_partial_initialization(place, statement.span, context);
    report_write_conflicts(place, statement.span, context);
    unknown
}

/// Rejects a move that leaves a value its type cannot give up.
///
/// A type with a declared destructor drops as a whole, so one field cannot leave
/// it without breaking the destructor's view of the value. An array owns a fixed
/// sequence of elements, so moving one element out would leave a hole the array
/// cannot describe. The by-value `for` loop moves elements through its own
/// statement and never reaches this check.
fn check_move(place: &Place, span: Span, context: &Context<'_>) {
    let tcx = context.hir.ty_ctxt();
    let mut prefix = Place::with_root(place.root, Vec::new());
    for projection in &place.projections {
        let current = origins::place_ty(context.hir, context.body, &prefix);
        match projection {
            seal_mir::Projection::Deref | seal_mir::Projection::RawDeref => return,
            seal_mir::Projection::Field(_) | seal_mir::Projection::VariantField { .. } => {
                if let TyKind::Adt { adt, .. } = tcx.kind(current)
                    && tcx.drop_method(adt).is_some()
                {
                    context.reporter.move_out_of_drop_type(current, span);
                    return;
                }
            }
            seal_mir::Projection::Index { .. } => {
                if let TyKind::Array { element, .. } = tcx.kind(current)
                    && !tcx.is_copy_with(element, &context.hir.param_env(context.body.owner))
                {
                    context.reporter.move_out_of_array(place, span);
                    return;
                }
            }
            seal_mir::Projection::TupleField(_) => {}
        }
        prefix.projections.push(*projection);
    }
}

/// Rejects initializing one field of a value that needs a destructor.
///
/// Such a value must be whole before it can be dropped: its glue runs the
/// destructor and then visits every field. Writing one field at a time would let
/// the destructor observe a half-built value, and a path that stops halfway
/// would leave a partially drop-able value behind.
fn check_partial_initialization(place: &Place, span: Span, context: &Context<'_>) {
    if place.is_whole() || place.is_deref() {
        return;
    }
    let Some(local) = place.local_id() else {
        return;
    };
    let root = context.body.local(local).ty;
    if !context.hir.ty_ctxt().needs_drop(root) {
        return;
    }
    let root_place = Place::local(local);
    if init::state_of(context.init, &root_place, context.hir.ty_ctxt(), root) != Init::Initialized {
        context.reporter.initialize_field_of_drop_type(place, span);
    }
}

/// Checks a drop statement's access to its place.
///
/// A lexical or explicit drop consumes the value, so it conflicts with every
/// live loan of the place exactly like a move does. An overwrite only takes
/// exclusive access: the value stored there is being replaced, so neither its
/// initialization nor its contents are observed.
fn check_drop(place: &Place, kind: DropKind, span: Span, context: &Context<'_>) -> bool {
    if matches!(kind, DropKind::Explicit) {
        check_read(place, span, context);
        // An explicit drop consumes the place like a move, so a field of a
        // type that implements `Drop` cannot leave it this way either.
        check_move(place, span, context);
    }
    if matches!(kind, DropKind::Overwrite) {
        report_write_conflicts(place, span, context);
    } else {
        report_move_conflicts(place, span, context);
    }
    context.keys_of(place).is_unknown()
}

/// Checks the statement that drops the elements a by-value loop did not move.
///
/// The array is consumed, so the drop conflicts with every live loan of it. It
/// reads no element value: the elements below `from` were moved out, which is
/// why the statement exists, and the initialization analysis cannot express
/// "the range after this position" for a runtime index.
fn check_drop_remaining(
    place: &Place,
    from: &Operand,
    statement: &Statement,
    context: &Context<'_>,
) -> bool {
    check_operand(statement, from, context);
    report_move_conflicts(place, statement.span, context);
    context.keys_of(place).is_unknown() || operand_unknown(from, context)
}

/// Reports a write that overlaps a live loan of `place`.
fn report_write_conflicts(place: &Place, span: Span, context: &Context<'_>) {
    if place_is_reported(context, place) {
        return;
    }
    for id in overlapping(context, place) {
        let loan = context.loan(id);
        if loan.kind.is_exclusive() {
            context
                .reporter
                .write_while_exclusively_borrowed(place, span, loan);
        } else {
            context.reporter.assign_while_borrowed(place, span, loan);
        }
    }
}

/// Reports a move that overlaps a live loan of `place`.
fn report_move_conflicts(place: &Place, span: Span, context: &Context<'_>) {
    if place.is_deref() || place_is_reported(context, place) {
        return;
    }
    for id in overlapping(context, place) {
        let loan = context.loan(id);
        if loan.kind.is_exclusive() {
            context
                .reporter
                .move_while_exclusively_borrowed(place, span, loan);
        } else {
            context.reporter.move_while_borrowed(place, span, loan);
        }
    }
}

/// Returns whether reading `operand` reaches storage through an unknown target.
fn operand_unknown(operand: &Operand, context: &Context<'_>) -> bool {
    operand
        .place()
        .is_some_and(|place| context.keys_of(&place).is_unknown())
}

/// Checks an operand's read and, when it moves, its conflict with live loans.
fn check_operand(statement: &Statement, operand: &Operand, context: &Context<'_>) {
    let Some(read) = operand.place() else {
        return;
    };
    check_read(&read, statement.span, context);

    if matches!(operand, Operand::Move(_)) {
        check_move(&read, statement.span, context);
        report_move_conflicts(&read, statement.span, context);
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
    let Some(local) = place.local_id() else {
        return place.clone();
    };
    (0..=place.projections.len())
        .map(|length| place.projections[..length].to_vec())
        .find_map(|projections| {
            let prefix = Place::with_projections(local, projections);
            matches!(state(context, &prefix), Init::Moved).then_some(prefix)
        })
        .unwrap_or_else(|| place.clone())
}

/// A shared dynamic call reads the pointee for the duration of the call.
///
/// The check is expressed as a read of the pointee place, so an exclusive loan
/// of the same storage conflicts. An exclusive call is handled by the call-site
/// argument rules instead.
fn check_shared_dynamic_receiver(receiver: &Operand, span: seal_span::Span, context: &Context<'_>) {
    let Some(place) = receiver.place() else {
        return;
    };
    let ty = origins::place_ty(context.hir, context.body, &place);
    if matches!(
        context.hir.ty_ctxt().kind(ty),
        TyKind::Ref {
            kind: RefKind::Shared,
            ..
        }
    ) {
        check_read(&place.deref(), span, context);
    }
}

fn state(context: &Context<'_>, place: &Place) -> Init {
    let root_ty = match place.local_id() {
        Some(local) => context.body.local(local).ty,
        None => context.hir.ty_ctxt().error_ty(),
    };
    init::state_of(context.init, place, context.hir.ty_ctxt(), root_ty)
}

/// Returns whether a loan targets a static or const allocation.
fn has_global_target(keys: &origins::OriginSet) -> bool {
    keys.only_globals()
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
    if let Some(local) = loan.place.local_id() {
        for held in holds.local_loans(local) {
            escaping_loans(holds, loans, held, visited, escaped);
        }
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
    let keys = context.keys_of(place);
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
                unknown = context.keys_of(&place).is_unknown();
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
            // A reference into a static or const allocation outlives the frame,
            // so only frame storage and untracked pointees can dangle. A
            // mutable static is rejected separately: its global origin is not
            // visible to later callees' effect summaries.
            let returned = context.origins.at(&Place::local(Body::return_local()));
            let mut escaped_statics = Vec::new();
            for key in returned.globals() {
                if let origins::StorageKey::Static { def, .. } = key
                    && context.hir.definitions().is_mutable(def)
                    && !escaped_statics.contains(&def)
                {
                    escaped_statics.push(def);
                }
            }
            escaped_statics.sort();
            for def in escaped_statics {
                context
                    .reporter
                    .return_mutable_static_reference(def, block.terminator.span);
            }
            escaped.sort_by_key(|id| id.index());
            for id in escaped {
                let loan = &context.loans[id.index() as usize];
                if loan.reported.get() || has_global_target(&loan.keys) {
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
