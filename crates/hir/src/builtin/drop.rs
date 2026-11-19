//! The compiler-provided `Drop` trait.
//!
//! `Drop` is the hook the compiler calls before the storage of a value is
//! released. It is reserved like the protocols so `impl Drop for Type` resolves
//! without a prelude, but the language narrows it beyond a general trait:
//! implementations are validated by [`Lowering::validate_drop_impls`], which
//! also records the destructor every later phase reads.
//!
//! [`Lowering::validate_drop_impls`]: crate::lower::Lowering::validate_drop_impls

use seal_def::DefId;
use seal_symbol::Interner;
use seal_ty::{RefKind, TraitMethodReceiver};

use super::{build_method, create_trait, install_trait_definition};
use crate::Hir;

/// The stable definition IDs of the compiler-provided `Drop` trait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DropDefs {
    /// The `Drop` trait declaration.
    pub(crate) trait_def: DefId,
    /// The `Drop::drop` method declaration.
    pub(crate) method: DefId,
}

/// Installs `Drop` into an empty HIR.
///
/// The trait has only its implicit `Self` parameter and declares one method,
/// `func drop(self: *mut Self)`, with no default body. The receiver is
/// exclusive: the destructor borrows the value that is about to be released and
/// never reads it by value.
pub(crate) fn install(hir: &mut Hir, interner: &Interner) -> DropDefs {
    let (trait_def, generics) = create_trait(hir, interner, "Drop", &[]);
    let unit = hir.ty_ctxt().unit_ty();
    let (method, definition) = build_method(
        hir,
        interner,
        trait_def,
        "drop",
        TraitMethodReceiver {
            reference: Some(RefKind::Exclusive),
        },
        Vec::new(),
        unit,
    );
    install_trait_definition(hir, interner, "Drop", trait_def, generics, vec![definition]);
    DropDefs { trait_def, method }
}
