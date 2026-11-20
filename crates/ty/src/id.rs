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
    /// different places. A type argument list is stored separately by
    /// [`TyKind::Adt`](crate::TyKind::Adt).
    pub struct AdtId;
}

newtype_index! {
    /// Index of a declared type parameter in a [`TyCtxt`](crate::TyCtxt).
    ///
    /// A parameter ID is unique to its declaration. Two parameters named `T`
    /// declared by different owners are distinct, even if their names and
    /// positions happen to match.
    pub struct ParamTyId;
}

newtype_index! {
    /// Index of an interned ordered list of type arguments.
    ///
    /// Substitutions are structural: `[i32, bool]` and `[bool, i32]` are
    /// different lists, while two requests for `[i32, bool]` share one ID.
    pub struct SubstsId;
}

newtype_index! {
    /// Index of a structural tuple type in a [`TyCtxt`](crate::TyCtxt).
    ///
    /// Tuples have no declaration site, so they are interned by their element
    /// list. Two tuple types are equal exactly when their `TupleId`s are equal,
    /// which makes the arity and every element part of type identity.
    pub struct TupleId;
}

newtype_index! {
    /// Index of an interned C function signature in a [`TyCtxt`](crate::TyCtxt).
    ///
    /// A function signature is structural: the ABI, the ordered parameter
    /// list, the return type, and variadicity decide identity. The signature is
    /// never stored by value; only a raw pointer to it has a representation.
    pub struct FnSigId;
}

newtype_index! {
    /// Index of an interned Seal function pointer in a [`TyCtxt`](crate::TyCtxt).
    ///
    /// A Seal function pointer is structural: the ordered parameter list, the
    /// return type, and the `unsafe` qualifier decide identity. Unlike a C
    /// signature, it describes a sized value with the internal Seal ABI.
    pub struct FnPtrId;
}
