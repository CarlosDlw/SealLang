//! Typed handles for AST arenas.
//!
//! Each node category has its own index type, so an expression ID cannot be
//! passed where a statement ID is expected. The IDs are dense and zero-based;
//! only the owning [`crate::Ast`] can resolve them.

use seal_index::newtype_index;

newtype_index! {
    /// Identifies an item in the program.
    pub struct ItemId;
}

newtype_index! {
    /// Identifies a function parameter.
    pub struct ParamId;
}

newtype_index! {
    /// Identifies a struct field declaration.
    pub struct FieldDeclId;
}

newtype_index! {
    /// Identifies a block.
    pub struct BlockId;
}

newtype_index! {
    /// Identifies a statement.
    pub struct StmtId;
}

newtype_index! {
    /// Identifies an expression.
    pub struct ExprId;
}

newtype_index! {
    /// Identifies a type.
    pub struct TypeId;
}
