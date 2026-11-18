//! The result of resolving a name.

use seal_def::DefId;
use seal_ty::TraitRef;

/// What a name resolved to.
///
/// Lowering never keeps a textual name for a resolved reference: consumers ask
/// the definition table for the details, which is what makes shadowing and
/// later renames irrelevant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Res {
    /// A function item.
    Function(DefId),
    /// An associated trait function whose implementation is selected later.
    ///
    /// A default associated function has no impl function of its own. The
    /// stored trait reference gives type checking the concrete `Self` and trait
    /// arguments needed to instantiate the declaration.
    TraitFunction {
        /// The trait function declaration.
        function: DefId,
        /// The trait reference it is called through.
        trait_ref: TraitRef,
    },
    /// A constant item or block-scope constant declaration.
    Const(DefId),
    /// A static item or block-scope static declaration.
    Static(DefId),
    /// A parameter or `let` binding.
    Local(DefId),
}

impl Res {
    /// Returns the definition this resolution refers to.
    #[inline]
    pub const fn def(self) -> DefId {
        match self {
            Self::Function(def) | Self::Const(def) | Self::Static(def) | Self::Local(def) => def,
            Self::TraitFunction { function, .. } => function,
        }
    }
}
