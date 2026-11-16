//! Loans: a borrow of a place for a region.

use seal_mir::Place;
use seal_span::Span;

/// One `&place` expression.
///
/// Every current loan is shared; an exclusive kind joins this struct with
/// `*mut T` and changes only the conflict table in [`crate::check`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct Loan {
    /// The borrowed place.
    pub(crate) place: Place,
    /// The source range of the borrow, used as a secondary label.
    pub(crate) span: Span,
}
