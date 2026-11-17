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

newtype_index! {
    /// Index of an enum variant inside its [`AdtDef`](crate::AdtDef).
    ///
    /// The index is also the discriminant stored in the value, so it stays
    /// stable for the whole compilation and drives layout, code generation, and
    /// the future `match` lowering.
    pub struct VariantIdx;
}

newtype_index! {
    /// Index of an aggregate definition in a [`TyCtxt`](crate::TyCtxt).
    ///
    /// The ID names the nominal definition, not the type: two fields with the
    /// same IDs denote the same aggregate even when their fields are written in
    /// different places.
    pub struct AdtId;
}

newtype_index! {
    /// Index of a structural tuple type in a [`TyCtxt`](crate::TyCtxt).
    ///
    /// Tuples have no declaration site, so they are interned by their element
    /// list. Two tuple types are equal exactly when their `TupleId`s are equal,
    /// which makes the arity and every element part of type identity.
    pub struct TupleId;
}
