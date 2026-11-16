//! Loans: a borrow of a place for a region.

use seal_mir::Place;
use seal_span::Span;
use seal_ty::RefKind;

/// One `&place` or `&mut place` expression.
///
/// The kind selects the conflict rules in [`crate::check`]: a shared loan
/// conflicts with writes and moves of its place, while an exclusive loan
/// conflicts with every other loan and with reads.
#[derive(Clone, Debug)]
pub(crate) struct Loan {
    /// The borrowed place.
    pub(crate) place: Place,
    /// The source range of the borrow, used as a secondary label.
    pub(crate) span: Span,
    /// Whether the borrow is shared or exclusive.
    pub(crate) kind: RefKind,
}
