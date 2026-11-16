//! The result of resolving a name.

use seal_def::DefId;

/// What a name resolved to.
///
/// Lowering never keeps a textual name for a resolved reference: consumers ask
/// the definition table for the details, which is what makes shadowing and
/// later renames irrelevant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Res {
    /// A function item.
    Function(DefId),
    /// A parameter or `let` binding.
    Local(DefId),
}

impl Res {
    /// Returns the definition this resolution refers to.
    #[inline]
    pub const fn def(self) -> DefId {
        match self {
            Self::Function(def) | Self::Local(def) => def,
        }
    }
}
