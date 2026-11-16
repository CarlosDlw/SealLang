//! Handles for interned types and inference variables.

use seal_index::newtype_index;

newtype_index! {
    /// Index of an interned type in a [`TyCtxt`](crate::TyCtxt).
    ///
    /// IDs are dense and only meaningful for the context that created them.
    pub struct TyId;
}

newtype_index! {
    /// Index of an inference variable in an [`InferCtxt`](crate::InferCtxt).
    pub struct InferTyId;
}
