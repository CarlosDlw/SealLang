//! Ownership and borrow checking over MIR.
//!
//! The checker enforces the memory model's invariants on explicit MIR facts:
//! every read is initialized, a moved place is not used again, a place is not
//! written or moved while a reference to it is live, and a reference to a
//! function-local place never escapes.
//!
//! # Regions
//!
//! Seal has no lifetime syntax (decision D5 of the memory model), so regions are
//! inferred. The checker runs three dataflow passes per body:
//!
//! - **Liveness** (backward): the set of locals that may still be read from each
//!   program point.
//! - **May-hold** (forward): for each local and point, the set of loans its
//!   reference value may carry. A loan is created by [`Rvalue::Ref`] and follows
//!   copies of the reference, so aliases keep the original loan alive.
//! - **Initialization** (forward): whether each local is uninitialized,
//!   initialized, or moved at each point.
//!
//! A loan is *active* at a point when some local that is live there may hold it.
//! This is flow sensitive in a way region-union inference is not: overwriting a
//! reference drops the loans it carried, so a place becomes writable again as
//! soon as the last alias is dead, even inside a loop. The analysis is the
//! location-sensitive shape of Polonius, applied to reference loans.
//!
//! # Loan kinds
//!
//! `*T` is read-only (decision D4) and `*mut T` is exclusive and read-write
//! (decision D6). A shared loan conflicts only with a write or a move of the
//! borrowed place; two shared loans may overlap, and reads never conflict. An
//! exclusive loan conflicts with every other loan of the same place, including
//! another shared loan, and with reads, writes, and moves of it.
//!
//! # Pointee aliasing
//!
//! Conflicts compare storage, not place syntax: `*p` and `*q` denote the same
//! storage when `p` and `q` may point at the same place. A parallel flow-
//! sensitive analysis records the storage each reference path may name;
//! `&place` names `place`, copies share the set, and `&*pointer` names what the
//! pointer names. A reborrow records the loans that authorized it, so using the
//! reborrow is allowed while using the parent conflicts with it. A reference
//! read out of memory has an unknown target and conservatively conflicts with
//! every live loan of storage it might name.
//!
//! # Call sites
//!
//! Passing a reference to a call gives the callee access to its pointee. An
//! exclusive argument grants sole access for the whole call, so the checker
//! rejects a call whose exclusive argument overlaps a live loan (`E5009`) and a
//! call whose reference arguments may name the same storage when one of them is
//! exclusive (`E5010`). Argument storage is compared with the may-name
//! analysis, so copies and reborrows are ordered.
//!
//! # `noalias` facts
//!
//! The backend may only assume that an exclusive parameter does not alias when
//! the whole program upholds the grant. [`check`] therefore also returns
//! [`AliasingFacts`]: a function is included when its body never reaches storage
//! through a reference of unknown target or a raw-memory operation, and every
//! call site proves the exclusive contract. The facts gate an optimization; they
//! never decide whether a program is accepted.
//!
//! # Errors
//!
//! Diagnostics are `E5xxx` and are emitted through the caller's [`DiagCtxt`],
//! in block and statement order, so a session's output is deterministic.

#![warn(missing_docs)]

mod access;
mod aliasing;
mod calls;
mod check;
mod effects;
mod holds;
mod id;
mod liveness;
mod loan;
mod loans;
mod origins;
mod report;

#[cfg(test)]
mod tests;

use seal_const_eval::ConstProgram;
use seal_diagnostics::DiagCtxt;
use seal_hash::FxHashMap;
use seal_hir::Hir;
use seal_mir::{Body, Mir};
use seal_symbol::Interner;

pub use aliasing::AliasingFacts;

/// Checks every body in `mir` and returns the facts the backend needs to mark
/// exclusive-reference parameters `noalias`.
///
/// The caller must have lowered `mir` from a HIR tree that type checking
/// accepted, which guarantees that every place, edge, and operand is well
/// formed. `program` supplies the evaluated constants, so a reference copied
/// out of a const names its real allocation; `None` is only for tests that
/// build a body directly and contain no such reference. Errors are reported
/// through `dcx`; a body with an error still gets the rest of its checks, so
/// one mistake does not hide another. The facts are computed even when a
/// diagnostic was emitted, but the driver only reaches code generation when
/// there are no errors.
pub fn check(
    hir: &Hir,
    mir: &Mir,
    program: Option<&ConstProgram>,
    dcx: &DiagCtxt,
    interner: &Interner,
) -> AliasingFacts {
    let effects = effects::analyze(hir, mir);
    let mut calls = aliasing::CallSites::default();
    let mut bodies = FxHashMap::default();
    for body in mir.bodies().iter() {
        let facts = check::check_body(hir, body, program, &effects, dcx, interner, &mut calls);
        bodies.insert(body.owner, facts);
    }
    aliasing::facts(hir.ty_ctxt(), mir, &bodies, &calls)
}

/// Checks one body.
///
/// This is the unit [`check`] iterates over; it is public so later passes and
/// tests can analyze a single body, including one constructed directly. Call
/// sites inside the body are checked, but the alias facts are only meaningful
/// for the whole program and are therefore returned by [`check`] alone.
pub fn check_body(hir: &Hir, body: &Body, dcx: &DiagCtxt, interner: &Interner) {
    let mut calls = aliasing::CallSites::default();
    let effects = effects::Effects::default();
    check::check_body(hir, body, None, &effects, dcx, interner, &mut calls);
}
