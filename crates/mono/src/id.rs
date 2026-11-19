//! Dense handles for collected instances.

use seal_index::newtype_index;

newtype_index! {
    /// Position of a [`MonoMir`](crate::MonoMir) in a
    /// [`MonoProgram`](crate::MonoProgram).
    ///
    /// IDs follow discovery order, so two collections of the same source assign
    /// the same ID to the same instance.
    pub struct InstanceId;
}

newtype_index! {
    /// Position of a [`MonoVTable`](crate::MonoVTable) in a
    /// [`MonoProgram`](crate::MonoProgram).
    ///
    /// IDs follow first-coercion order and are independent from instance IDs.
    pub struct VtableId;
}

newtype_index! {
    /// Position of a
    /// [`DropGlueEntry`](crate::DropGlueEntry) in a
    /// [`MonoProgram`](crate::MonoProgram).
    ///
    /// IDs follow first-request order and are independent from instance and
    /// vtable IDs.
    pub struct DropGlueId;
}
