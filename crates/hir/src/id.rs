//! Handles for HIR nodes.

use seal_index::newtype_index;

newtype_index! {
    /// Index of an executable body in a [`Hir`](crate::Hir).
    pub struct BodyId;
}

newtype_index! {
    /// Index of a block in a [`Hir`](crate::Hir).
    pub struct HirBlockId;
}

newtype_index! {
    /// Index of a statement in a [`Hir`](crate::Hir).
    pub struct HirStmtId;
}

newtype_index! {
    /// Index of an expression in a [`Hir`](crate::Hir).
    pub struct HirExprId;
}
