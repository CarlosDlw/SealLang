//! The Seal mid-level intermediate representation.
//!
//! MIR is a control-flow graph per function. It is the input to move analysis,
//! borrow checking, drop elaboration, optimization, and code generation.
//!
//! # Shape
//!
//! Every body has an entry block and a return place (local zero). Parameters are
//! locals `1..=arg_count`; bindings and temporaries follow. Each block ends in a
//! [`Terminator`](crate::Terminator), so there are no implicit fallthrough
//! edges. Blocks whose control never reaches them are possible after a `return`
//! followed by unreachable source statements; validation does not require
//! reachability, and later passes may remove them.
//!
//! [`Rvalue::Call`](crate::Rvalue::Call) is a statement rather than a terminator:
//! Seal has no unwinding, so a call cannot leave the block on a failure path.
//! Borrows are [`Rvalue::Ref`](crate::Rvalue::Ref) statements, and reads through
//! a reference carry a dereference projection on the [`Place`](crate::Place).
//!
//! # Lowering
//!
//! [`lower`] consumes a checked [`Hir`](seal_hir::Hir) and its
//! [`TypeckResults`](seal_typeck::TypeckResults). It assumes type checking
//! reported no errors; the driver only runs it on a clean tree. Evaluation order
//! is preserved, `&&`/`||` become `SwitchInt` graphs so the right operand is
//! evaluated only when needed, `&place` becomes a `Ref`, and
//! `if`/`while`/`break`/`continue` become explicit edges.
//!
//! # Example
//!
//! ```ignore
//! let typeck = seal_typeck::check(&hir, session.diag(), session.interner(), 64);
//! let mir = seal_mir::lower(&hir, &typeck);
//! mir.validate().expect("MIR lowering maintains its invariants");
//! ```

#![warn(missing_docs)]

mod body;
mod id;
mod lower;
mod mir;
mod operand;
mod place;
mod rvalue;
mod statement;
mod terminator;
#[cfg(test)]
mod tests;
mod validate;

pub use body::{BasicBlock, Body, LocalDecl, LocalKind};
pub use id::{BasicBlockId, BodyId, LocalId};
pub use lower::lower;
pub use mir::Mir;
pub use operand::{Constant, Operand};
pub use place::{Place, Projection};
pub use rvalue::{AggregateKind, BinOp, Rvalue, UnOp};
pub use statement::{Statement, StatementKind};
pub use terminator::{SwitchTarget, Terminator, TerminatorKind};
pub use validate::ValidationError;
