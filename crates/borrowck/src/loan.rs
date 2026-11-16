//! Loans: a borrow of a place for a region.

use seal_mir::Place;
use seal_span::Span;
use seal_ty::RefKind;

use crate::id::LoanId;
use crate::origins::OriginSet;

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
    /// The storage the borrowed place may name, resolved when the loan is
    /// created.
    pub(crate) keys: OriginSet,
    /// The loans that authorized the borrowed place, when it was reached
    /// through a reference.
    ///
    /// A reborrow is derived from those loans, so using it does not conflict
    /// with them; using the parent loan conflicts with the reborrow.
    pub(crate) parents: Vec<LoanId>,
    /// The last program point where the loan is active, used to explain a
    /// conflict.
    pub(crate) last_use: Option<Span>,
}
