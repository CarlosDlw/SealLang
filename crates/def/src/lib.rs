//! Definition identities shared by every semantic phase.
//!
//! A [`DefId`] names something that can be referenced: a function, a parameter,
//! or a `let` binding. Later phases use IDs instead of names, so a lookup
//! happens once and every consumer sees the same definition even if the name is
//! shadowed.
//!
//! Definitions are never renumbered or reused. Adding a definition appends to
//! the arena, which keeps IDs stable for diagnostics, side tables keyed by
//! `DefId`, and future incremental compilation.
//!
//! Every definition also records the package, package instance, module, and
//! visibility that declare it. A local definition such as a parameter records
//! the module of its owning item, so a consumer can always reconstruct the
//! visibility boundary without walking parents.

#![warn(missing_docs)]

mod definitions;
mod id;
mod kind;
mod visibility;

pub use definitions::{DefData, Definitions};
pub use id::{DefId, ModuleId, PackageId, PackageInstanceId};
pub use kind::DefKind;
pub use visibility::Visibility;
