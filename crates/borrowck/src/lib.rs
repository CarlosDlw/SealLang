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
//! another shared loan, and with reads, writes, and moves of it. Accesses
//! through a dereference are not compared with loans: the checker does not
//! track aliasing into borrowed data, because the pointer may have been copied.
//!
//! # Errors
//!
//! Diagnostics are `E5xxx` and are emitted through the caller's [`DiagCtxt`],
//! in block and statement order, so a session's output is deterministic.

#![warn(missing_docs)]

mod access;
mod check;
mod holds;
mod id;
mod init;
mod liveness;
mod loan;
mod loans;
mod report;

#[cfg(test)]
mod tests;

use seal_diagnostics::DiagCtxt;
use seal_hir::Hir;
use seal_mir::{Body, Mir};
use seal_symbol::Interner;

/// Checks every body in `mir`.
///
/// The caller must have lowered `mir` from a HIR tree that type checking
/// accepted, which guarantees that every place, edge, and operand is well
/// formed. Errors are reported through `dcx`; a body with an error still gets
/// the rest of its checks, so one mistake does not hide another.
pub fn check(hir: &Hir, mir: &Mir, dcx: &DiagCtxt, interner: &Interner) {
    for body in mir.bodies().iter() {
        check_body(hir, body, dcx, interner);
    }
}

/// Checks one body.
///
/// This is the unit [`check`] iterates over; it is public so later passes and
/// tests can analyze a single body, including one constructed directly.
pub fn check_body(hir: &Hir, body: &Body, dcx: &DiagCtxt, interner: &Interner) {
    check::check_body(hir, body, dcx, interner);
}
