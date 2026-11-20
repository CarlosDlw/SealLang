//! Drop elaboration: making every drop site executable.
//!
//! Lowering inserts a drop site where a value's scope ends, but it cannot know
//! whether a value is still there at that point. Elaboration answers that with
//! two analyses and then rewrites the body:
//!
//! - A site whose place is definitely initialized drops unconditionally.
//! - A site whose place is definitely empty on every path is removed. This is
//!   what keeps a constant from reporting a destructor it never runs: a
//!   temporary moved into the return value leaves nothing to drop, and constant
//!   evaluation only rejects the drops that would actually execute.
//! - Every other site is guarded by a `bool` flag that is set when a value is
//!   written and cleared when one is moved or dropped, so the value is dropped
//!   exactly when the executing path left one there.
//!
//! One drop site can cover several values. A struct without a destructor drops
//! its fields one at a time in declaration order, while an enum, an array, and a
//! type with a declared destructor are dropped by their glue as one unit; the
//! expansion lives in [`tree`].
//!
//! Finally, an assignment to an initialized place whose type needs a destructor
//! drops the value it replaces first ([`overwrite`]).
//!
//! Elaboration runs once per tree, before borrow checking and constant
//! evaluation. A body that already carries drop flags is left untouched, so a
//! repeated run cannot nest guards; a body without any flag has only
//! unconditional sites and is stable because a replaced value is not dropped
//! twice.

mod dead;
mod flags;
mod overwrite;
mod tree;

use std::fmt;

use seal_def::DefId;
use seal_hir::Hir;
use seal_index::Idx;
use seal_span::Span;
use seal_ty::TyId;

use crate::body::{BasicBlock, Body, LocalKind};
use crate::id::{BasicBlockId, LocalId};
use crate::init::{self, Init, InitInfo};
use crate::mir::Mir;
use crate::operand::{Constant, Operand};
use crate::place::Place;
use crate::rvalue::{IntrinsicReceiver, Rvalue};
use crate::statement::{DropKind, Statement, StatementKind};
use crate::terminator::{SwitchTarget, Terminator, TerminatorKind};

use self::flags::{FlagKey, Flags};

/// A body that drop elaboration could not process.
///
/// The variants describe invariants the earlier phases establish, so the driver
/// reports them as compiler bugs instead of user errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ElaborationError {
    /// A place names a local the body does not declare.
    UnknownLocal {
        /// The body that holds the place.
        owner: DefId,
        /// The local that has no declaration.
        local: LocalId,
    },
    /// The initialization analysis has no state for a statement of the body.
    MissingInitState {
        /// The body being elaborated.
        owner: DefId,
        /// The block whose state is missing.
        block: BasicBlockId,
    },
    /// A planned guard names a flag that was not declared.
    MissingFlag {
        /// The body being elaborated.
        owner: DefId,
        /// The local the missing flag tracked.
        local: LocalId,
    },
}

impl fmt::Display for ElaborationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownLocal { owner, local } => write!(
                formatter,
                "drop elaboration reached local {local:?} of {owner:?}, which the body does not declare"
            ),
            Self::MissingInitState { owner, block } => write!(
                formatter,
                "drop elaboration has no initialization state for {block:?} of {owner:?}"
            ),
            Self::MissingFlag { owner, local } => write!(
                formatter,
                "drop elaboration planned a guard for local {local:?} of {owner:?} without declaring its flag"
            ),
        }
    }
}

/// Elaborates the drop sites of every body in `mir`.
///
/// Constants and statics are elaborated too: a dead drop site inside an
/// initializer must be removed before constant evaluation runs, or it would be
/// rejected as a destructor call that never happens.
pub fn elaborate_drops(hir: &Hir, mir: &mut Mir) -> Result<(), ElaborationError> {
    for body in mir.bodies.iter_mut() {
        elaborate_body(hir, body)?;
    }
    Ok(())
}

/// Elaborates one body.
fn elaborate_body(hir: &Hir, body: &mut Body) -> Result<(), ElaborationError> {
    if body.blocks.is_empty() || is_elaborated(body) {
        return Ok(());
    }
    let init = init::analyze(hir, body);
    let plan = plan_body(hir, body, &init)?;

    let bool_ty = hir.ty_ctxt().bool_ty();
    let mut flags = Flags::default();
    for request in &plan.requests {
        flags.declare(body, request.key.clone(), bool_ty, request.span);
    }
    // The plan is indexed by the statements of the body before elaboration, so
    // the original blocks are captured before the entry flags are prepended.
    let originals: Vec<BasicBlock> = body.blocks.iter().cloned().collect();
    let entry_prefix = entry_flag_assignments(body, &flags);
    rewrite_body(body, &originals, &entry_prefix, &plan.statements, &flags)
}

/// Returns whether elaboration already ran over this body.
///
/// Drop flags are compiler storage that no earlier phase creates, so their
/// presence is the marker. Leaving such a body alone keeps a repeated run from
/// wrapping an existing guard in a second one.
fn is_elaborated(body: &Body) -> bool {
    body.locals
        .iter()
        .any(|declaration| declaration.kind == LocalKind::DropFlag)
}

/// Returns the declared type of `local`.
fn local_ty(body: &Body, local: LocalId) -> Result<TyId, ElaborationError> {
    if (local.index() as usize) >= body.locals.len() {
        return Err(ElaborationError::UnknownLocal {
            owner: body.owner,
            local,
        });
    }
    Ok(body.local(local).ty)
}

/// Returns the type the place starts from, which resolves a projected place.
fn root_ty(hir: &Hir, body: &Body, place: &Place) -> Result<TyId, ElaborationError> {
    match place.local_id() {
        Some(local) => local_ty(body, local),
        None => Ok(hir.ty_ctxt().error_ty()),
    }
}

/// A drop elaboration requests for one body.
struct Plan {
    /// What replaces each statement, indexed by block and position.
    statements: Vec<Vec<Action>>,
    /// The flags the plan needs, in the order the sites asked for them.
    requests: Vec<FlagRequest>,
}

/// A flag a plan needs.
struct FlagRequest {
    key: FlagKey,
    span: Span,
}

/// What happens to one statement.
enum Action {
    /// A drop site: the statement is replaced by these drops.
    Site(Vec<PlannedDrop>),
    /// A statement that stays, preceded by these drops.
    Keep { drops: Vec<PlannedDrop> },
}

/// One drop a statement performs.
struct PlannedDrop {
    /// The statement that drops the value.
    drop: Statement,
    /// How the statement decides whether the value is there.
    route: Route,
}

/// How a drop decides whether its value is present.
enum Route {
    /// The value is definitely there.
    Always,
    /// The value is there when the place's flag is set.
    Guarded(FlagKey),
}

/// Builds the plan for one body without touching it.
fn plan_body(hir: &Hir, body: &Body, init: &InitInfo) -> Result<Plan, ElaborationError> {
    let mut requests = Vec::new();
    let mut statements = Vec::with_capacity(body.blocks.len());
    for (block_index, block) in body.blocks.iter_enumerated() {
        let before = init.before.get(block_index.index() as usize).ok_or(
            ElaborationError::MissingInitState {
                owner: body.owner,
                block: block_index,
            },
        )?;
        let mut row = Vec::with_capacity(block.statements.len());
        for (position, statement) in block.statements.iter().enumerate() {
            let previous = position
                .checked_sub(1)
                .map(|index| &block.statements[index]);
            row.push(plan_statement(
                hir,
                body,
                before,
                block_index.index() as usize,
                position,
                statement,
                previous,
                &mut requests,
            )?);
        }
        statements.push(row);
    }
    Ok(Plan {
        statements,
        requests,
    })
}

/// Plans one statement.
///
/// `previous` is the statement before it in the same block, which lets the
/// overwrite rule recognize the drop an earlier elaboration inserted.
#[allow(clippy::too_many_arguments)]
fn plan_statement(
    hir: &Hir,
    body: &Body,
    before: &[seal_ty::InitPaths<LocalId>],
    block: usize,
    position: usize,
    statement: &Statement,
    previous: Option<&Statement>,
    requests: &mut Vec<FlagRequest>,
) -> Result<Action, ElaborationError> {
    match &statement.kind {
        StatementKind::Drop { place, kind } => {
            // An overwrite drop reached here only from an earlier elaboration,
            // which `is_elaborated` normally rules out; keeping it unchanged is
            // the safe reading either way.
            if *kind == DropKind::Overwrite {
                return Ok(Action::Keep { drops: Vec::new() });
            }
            let mut drops = Vec::new();
            for node in tree::drop_nodes(hir, body, place) {
                if let Some(planned) = plan_place(
                    hir,
                    body,
                    before,
                    block,
                    position,
                    statement.span,
                    &node,
                    *kind,
                    requests,
                )? {
                    drops.push(planned);
                }
            }
            Ok(Action::Site(drops))
        }
        StatementKind::Assign { place, rvalue } => {
            let mut drops = Vec::new();
            if overwrite::must_drop_old_value(hir, body, place)
                && !overwrite::takes_value_from_destination(rvalue, place)
                && !already_replaced(previous, place)
                && let Some(planned) = plan_place(
                    hir,
                    body,
                    before,
                    block,
                    position,
                    statement.span,
                    place,
                    DropKind::Overwrite,
                    requests,
                )?
            {
                drops.push(planned);
            }
            Ok(Action::Keep { drops })
        }
        StatementKind::DropRemaining { .. } => Ok(Action::Keep { drops: Vec::new() }),
    }
}

/// Returns whether `previous` already drops `place` for the assignment.
fn already_replaced(previous: Option<&Statement>, place: &Place) -> bool {
    matches!(
        previous.map(|statement| &statement.kind),
        Some(StatementKind::Drop {
            place: dropped,
            kind: DropKind::Overwrite,
        }) if dropped == place
    )
}

/// Decides how one value is dropped.
///
/// Returns `None` when the value is definitely gone, which removes the drop.
#[allow(clippy::too_many_arguments)]
fn plan_place(
    hir: &Hir,
    body: &Body,
    before: &[seal_ty::InitPaths<LocalId>],
    block: usize,
    position: usize,
    span: Span,
    place: &Place,
    kind: DropKind,
    requests: &mut Vec<FlagRequest>,
) -> Result<Option<PlannedDrop>, ElaborationError> {
    let Some(state) = before.get(position) else {
        return Err(ElaborationError::MissingInitState {
            owner: body.owner,
            block: BasicBlockId::from_usize(block),
        });
    };
    if let Some(local) = place.local_id() {
        local_ty(body, local)?;
    }
    let dead = dead::analyze(body, place);
    if dead.before(block, position) {
        return Ok(None);
    }
    let state = init::state_of(state, place, hir.ty_ctxt(), root_ty(hir, body, place)?);
    let drop = Statement::drop(place.clone(), kind, span);
    if state == Init::Initialized {
        return Ok(Some(PlannedDrop {
            drop,
            route: Route::Always,
        }));
    }
    // A place behind a reference always holds a value, so only frame storage
    // reaches here. Without a path there is nothing to track, and dropping is
    // still the correct reading of a place the analysis called initialized.
    let Some(key) = FlagKey::of(place) else {
        return Ok(Some(PlannedDrop {
            drop,
            route: Route::Always,
        }));
    };
    requests.push(FlagRequest {
        key: key.clone(),
        span,
    });
    Ok(Some(PlannedDrop {
        drop,
        route: Route::Guarded(key),
    }))
}

/// Returns the flag values every local starts with.
///
/// A parameter holds a value at entry, so a flag under one starts set; every
/// other local starts empty. The rewrite emits these at the top of the entry
/// block, ahead of the body's own statements.
fn entry_flag_assignments(body: &Body, flags: &Flags) -> Vec<Statement> {
    flags
        .entries()
        .into_iter()
        .map(|(flag, key)| {
            let initialized = body.local(key.local).kind == LocalKind::Arg;
            flag_assignment(flag, initialized, body.local(flag).span)
        })
        .collect()
}

/// Rewrites the body from its plan.
///
/// Blocks are rebuilt in place: the original block keeps the statements up to
/// the first guard, and every conditional drop appends a guard block and a
/// continuation block to the end of the body. Existing block IDs never change,
/// so every terminator that already pointed at a block still does.
fn rewrite_body(
    body: &mut Body,
    originals: &[BasicBlock],
    entry_prefix: &[Statement],
    plan: &[Vec<Action>],
    flags: &Flags,
) -> Result<(), ElaborationError> {
    for (index, original) in originals.iter().enumerate() {
        let block = BasicBlockId::from_usize(index);
        let mut current = block;
        let mut pending: Vec<Statement> = if block == body.entry {
            entry_prefix.to_vec()
        } else {
            Vec::new()
        };

        for (position, statement) in original.statements.iter().enumerate() {
            let action = plan.get(index).and_then(|row| row.get(position)).ok_or(
                ElaborationError::MissingInitState {
                    owner: body.owner,
                    block,
                },
            )?;

            // A move ends the value before the statement that performs it runs.
            if let StatementKind::Assign { rvalue, .. } = &statement.kind {
                for moved in moved_places(rvalue) {
                    clear_flags(flags, moved, statement.span, &mut pending);
                }
            }

            match action {
                Action::Site(drops) => {
                    for planned in drops {
                        emit_drop(body, flags, planned, &mut pending, &mut current)?;
                    }
                    if let StatementKind::Drop { place, .. } = &statement.kind {
                        clear_flags(flags, place, statement.span, &mut pending);
                    }
                }
                Action::Keep { drops } => {
                    for planned in drops {
                        emit_drop(body, flags, planned, &mut pending, &mut current)?;
                    }
                    pending.push(statement.clone());
                    if let StatementKind::Assign { place, .. } = &statement.kind {
                        raise_flags(flags, place, statement.span, &mut pending);
                    }
                    if let StatementKind::DropRemaining { place, .. } = &statement.kind {
                        clear_flags(flags, place, statement.span, &mut pending);
                    }
                }
            }
        }

        body.blocks[current].statements = pending;
        body.blocks[current].terminator = original.terminator.clone();
    }
    Ok(())
}

/// Emits one planned drop, splitting the block when it is guarded.
fn emit_drop(
    body: &mut Body,
    flags: &Flags,
    planned: &PlannedDrop,
    pending: &mut Vec<Statement>,
    current: &mut BasicBlockId,
) -> Result<(), ElaborationError> {
    let span = planned.drop.span;
    let flag = match &planned.route {
        Route::Always => {
            pending.push(planned.drop.clone());
            return Ok(());
        }
        Route::Guarded(key) => flags.get(key).ok_or(ElaborationError::MissingFlag {
            owner: body.owner,
            local: key.local,
        })?,
    };

    let continuation = body.blocks.push(BasicBlock {
        statements: Vec::new(),
        terminator: Terminator {
            kind: TerminatorKind::Unreachable,
            span,
        },
    });
    let guard = body.blocks.push(BasicBlock {
        statements: vec![planned.drop.clone()],
        terminator: Terminator {
            kind: TerminatorKind::Goto(continuation),
            span,
        },
    });
    body.blocks[*current].statements = std::mem::take(pending);
    body.blocks[*current].terminator = Terminator {
        kind: TerminatorKind::SwitchInt {
            discriminant: Operand::Copy(Place::local(flag)),
            targets: vec![SwitchTarget {
                value: 1,
                target: guard,
            }],
            otherwise: continuation,
        },
        span,
    };
    *current = continuation;
    Ok(())
}

/// Appends the assignments that clear the flags a moved or dropped place ends.
fn clear_flags(flags: &Flags, place: &Place, span: Span, pending: &mut Vec<Statement>) {
    let Some(key) = FlagKey::of(place) else {
        return;
    };
    for flag in flags.cleared(&key) {
        pending.push(flag_assignment(flag, false, span));
    }
}

/// Appends the assignments that set the flags an assignment initializes.
fn raise_flags(flags: &Flags, place: &Place, span: Span, pending: &mut Vec<Statement>) {
    let Some(key) = FlagKey::of(place) else {
        return;
    };
    for flag in flags.raised(&key) {
        pending.push(flag_assignment(flag, true, span));
    }
}

/// Creates the assignment of a literal to a drop flag.
fn flag_assignment(flag: LocalId, value: bool, span: Span) -> Statement {
    Statement::assign(
        Place::local(flag),
        Rvalue::Use(Operand::Constant(Constant::Bool(value))),
        span,
    )
}

/// Returns every place an rvalue moves out of, in evaluation order.
fn moved_places(rvalue: &Rvalue) -> Vec<&Place> {
    let mut places = Vec::new();
    match rvalue {
        Rvalue::Use(operand) | Rvalue::Cast { operand, .. } => add_move(operand, &mut places),
        Rvalue::Binary { lhs, rhs, .. } => {
            add_move(lhs, &mut places);
            add_move(rhs, &mut places);
        }
        Rvalue::Unary { operand, .. } => add_move(operand, &mut places),
        Rvalue::Call { args, .. } => {
            for arg in args {
                add_move(arg, &mut places);
            }
        }
        Rvalue::IndirectCall { callee, args, .. } => {
            add_move(callee, &mut places);
            for arg in args {
                add_move(arg, &mut places);
            }
        }
        Rvalue::Intrinsic { receiver, args, .. } => {
            if let IntrinsicReceiver::Value(operand) = receiver {
                add_move(operand, &mut places);
            }
            for arg in args {
                add_move(arg, &mut places);
            }
        }
        Rvalue::Aggregate { fields, .. } => {
            for (_, operand) in fields {
                add_move(operand, &mut places);
            }
        }
        Rvalue::Tuple { elements, .. } | Rvalue::Array { elements, .. } => {
            for element in elements {
                add_move(element, &mut places);
            }
        }
        Rvalue::Repeat { value, .. } => add_move(value, &mut places),
        Rvalue::Len { slice } => add_move(slice, &mut places),
        Rvalue::MakeTraitObject { source, .. } => add_move(source, &mut places),
        Rvalue::TraitObjectReborrow { source, .. } => add_move(source, &mut places),
        Rvalue::DynamicCall { receiver, args, .. } => {
            add_move(receiver, &mut places);
            for arg in args {
                add_move(arg, &mut places);
            }
        }
        Rvalue::Discriminant(_) | Rvalue::Ref { .. } | Rvalue::SliceRef { .. } => {}
    }
    places
}

/// Pushes the place `operand` moves, if it moves one.
fn add_move<'a>(operand: &'a Operand, places: &mut Vec<&'a Place>) {
    if let Operand::Move(place) = operand {
        places.push(place);
    }
}
