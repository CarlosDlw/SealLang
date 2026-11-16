//! Handles for MIR entities.

use seal_index::newtype_index;

newtype_index! {
    /// Index of a function body in a [`Mir`](crate::Mir).
    pub struct BodyId;
}

newtype_index! {
    /// Index of a basic block in a [`Body`](crate::Body).
    pub struct BasicBlockId;
}

newtype_index! {
    /// Index of a local in a [`Body`](crate::Body).
    pub struct LocalId;
}
