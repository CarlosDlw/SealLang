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
//! Its [`CallTarget`](crate::CallTarget) keeps the complete instance: a concrete
//! call names the function or impl method with its type arguments, while a call
//! through a trait bound names the trait method and stays symbolic until
//! monomorphization selects the implementation. Aggregate literals carry the
//! instantiation's type arguments too, so a generic body can resolve every field
//! projection from its own parameter types.
//! Borrows are [`Rvalue::Ref`](crate::Rvalue::Ref) statements, and reads through
//! a reference carry a dereference projection on the [`Place`](crate::Place). A
//! dereference whose operand is itself a value, as in `*identity(&v)` or `*&x`,
//! lowers that reference into a temporary first: HIR calls the expression a
//! value rather than a place, but it still denotes storage.
//!
//! # Lowering
//!
//! [`lower`] consumes a checked [`Hir`](seal_hir::Hir) and its
//! [`TypeckResults`](seal_typeck::TypeckResults). It assumes type checking
//! reported no errors; the driver only runs it on a clean tree. Generic bodies
//! lower like any other: parameters stay rigid, `Copy` decisions consult the
//! body's parameter environment, and field types are resolved after substituting
//! the instantiation's arguments. Evaluation order
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
mod drop_elaboration;
mod id;
pub mod init;
mod instantiate;
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

pub use drop_elaboration::{ElaborationError, elaborate_drops};
pub use instantiate::instantiate_body;

pub use body::{BasicBlock, Body, BodyKind, LocalDecl, LocalKind};
pub use id::{BasicBlockId, BodyId, LocalId};
pub use lower::{lower, lower_body, lower_body_with_declarations};
pub use mir::Mir;
pub use operand::{Constant, Operand};
pub use place::{Place, PlaceRoot, Projection};
pub use rvalue::{AggregateKind, BinOp, CallTarget, IntrinsicReceiver, Rvalue, TypeInfoKind, UnOp};
pub use statement::{DropKind, Statement, StatementKind};
pub use terminator::{SwitchTarget, Terminator, TerminatorKind};
pub use validate::ValidationError;
pub use validate::place_ty;
