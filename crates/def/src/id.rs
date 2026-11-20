//! Handles for definitions, packages, and modules.

use seal_index::newtype_index;

newtype_index! {
    /// Index of a definition in a [`Definitions`](crate::Definitions) table.
    ///
    /// IDs are dense, four bytes, and only meaningful for the table that
    /// created them.
    pub struct DefId;
}

newtype_index! {
    /// The identity of one package in a compilation plan.
    ///
    /// A package is a unit of distribution: it has one root module, its own
    /// visibility boundary, and its own implementation coherence rules.
    pub struct PackageId;
}

newtype_index! {
    /// The identity of one compiled instance of a package.
    ///
    /// A package can be compiled more than once with different features or
    /// configuration. The instance distinguishes those compilations while the
    /// [`PackageId`] keeps the source identity.
    pub struct PackageInstanceId;
}

newtype_index! {
    /// The identity of one module in a package.
    ///
    /// Modules form a tree rooted at the package root. IDs are global to the
    /// compilation, so a module always identifies its package without a lookup
    /// through another table.
    pub struct ModuleId;
}
