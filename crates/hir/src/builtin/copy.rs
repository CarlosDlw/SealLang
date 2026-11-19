//! The compiler-provided `Copy` trait.
//!
//! `Copy` is a marker: it has no methods and no default body. The language
//! reserves the name so `impl Copy for Type` resolves without a prelude, but
//! the property is not a normal trait contract. [`Lowering::validate_copy_impls`]
//! checks that the implementation covers the whole declaration, records it, and
//! removes it again if any field of any variant is not `Copy`.
//!
//! [`Lowering::validate_copy_impls`]: crate::lower::Lowering::validate_copy_impls

use seal_def::DefId;
use seal_symbol::Interner;

use super::{create_trait, install_trait_definition};
use crate::Hir;

/// The stable definition IDs of the compiler-provided `Copy` trait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CopyDefs {
    /// The `Copy` trait declaration.
    pub(crate) trait_def: DefId,
}

/// Installs `Copy` into an empty HIR.
///
/// The trait has only its implicit `Self` parameter and declares no methods.
pub(crate) fn install(hir: &mut Hir, interner: &Interner) -> CopyDefs {
    let (trait_def, generics) = create_trait(hir, interner, "Copy", &[]);
    install_trait_definition(hir, interner, "Copy", trait_def, generics, Vec::new());
    CopyDefs { trait_def }
}
