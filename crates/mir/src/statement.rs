//! MIR statements.

use seal_span::Span;

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
}
