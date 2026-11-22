//! MIR statements.

use seal_span::Span;

use crate::operand::Operand;
use crate::place::Place;
use crate::rvalue::Rvalue;

/// One statement inside a basic block.
#[derive(Clone, Debug, PartialEq)]
pub struct Statement {
    /// What the statement does.
    pub kind: StatementKind,
    /// The source range this statement came from.
    pub span: Span,
}

/// What a drop statement is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropKind {
    /// A drop site inserted by lowering for a lexical scope, an early exit, a
    /// parameter, or a temporary.
    ///
    /// The place may already be dead when the statement runs: a value can be
    /// moved out before the end of its scope, and deciding whether it is still
    /// there is drop elaboration's job. A dead place is therefore not a user
    /// error, and the statement does not read the place for diagnostics.
    Scope,
    /// The user's `drop(value)`.
    ///
    /// The value is consumed here, so dropping a place that was already moved
    /// is an error (`E5005`), exactly like any other move.
    Explicit,
    /// The value an assignment is about to replace.
    ///
    /// Drop elaboration inserts this immediately before an assignment to an
    /// initialized place whose type needs a destructor. It never changes
    /// initialization state: the following assignment writes the same place.
    Overwrite,
}

/// The kinds of statement.
#[derive(Clone, Debug, PartialEq)]
pub enum StatementKind {
    /// Evaluates `rvalue` and writes it to `place`.
    Assign {
        /// The destination.
        place: Place,
        /// The value.
        rvalue: Rvalue,
    },
    /// Drops the value stored in `place`.
    ///
    /// A drop takes exclusive access to the place and consumes the value: after
    /// a `Scope` or `Explicit` drop the place is moved (unless its type is
    /// `Copy`). Code generation calls the drop glue of the place's type with the
    /// place address; the glue runs a declared destructor and then drops the
    /// fields.
    Drop {
        /// The value being dropped.
        place: Place,
        /// Why the drop exists and how it consumes the place.
        kind: DropKind,
    },
    /// Consumes the value stored in `place` without running its destructor.
    ///
    /// The operand is a real move: the place becomes uninitialized, and every
    /// live loan of it conflicts exactly as it would with a drop. Unlike a
    /// drop, the statement runs no destructor, requests no drop glue, and
    /// generates no machine code.
    Forget {
        /// The value whose destruction obligation is cancelled.
        place: Place,
    },
    /// Drops the elements `from..length` of the array in `place`, in ascending
    /// index order.
    ///
    /// A by-value `for` loop moves its elements out through an exclusive borrow,
    /// which the initialization analysis cannot track per position. The loop
    /// therefore owns the array and drops only the elements it has not moved
    /// yet, which is exactly the range starting at `from`.
    DropRemaining {
        /// The array being consumed.
        place: Place,
        /// The first element that was not moved out.
        from: Operand,
    },
}

impl Statement {
    /// Creates an assignment statement.
    #[inline]
    pub fn assign(place: Place, rvalue: Rvalue, span: Span) -> Self {
        Self {
            kind: StatementKind::Assign { place, rvalue },
            span,
        }
    }

    /// Creates a drop statement.
    #[inline]
    pub fn drop(place: Place, kind: DropKind, span: Span) -> Self {
        Self {
            kind: StatementKind::Drop { place, kind },
            span,
        }
    }

    /// Creates a statement that consumes a value without dropping it.
    #[inline]
    pub fn forget(place: Place, span: Span) -> Self {
        Self {
            kind: StatementKind::Forget { place },
            span,
        }
    }

    /// Creates a statement that drops the remaining elements of an array.
    #[inline]
    pub fn drop_remaining(place: Place, from: Operand, span: Span) -> Self {
        Self {
            kind: StatementKind::DropRemaining { place, from },
            span,
        }
    }
}
