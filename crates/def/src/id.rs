//! Handles for definitions.

use seal_index::newtype_index;

newtype_index! {
    /// Index of a definition in a [`Definitions`](crate::Definitions) table.
    ///
    /// IDs are dense, four bytes, and only meaningful for the table that
    /// created them.
    pub struct DefId;
}
