//! Handles for loans.

use seal_index::newtype_index;

newtype_index! {
    /// Index of a loan in one body's loan table.
    pub(crate) struct LoanId;
}
