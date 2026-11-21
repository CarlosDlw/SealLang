//! Functions exported with a C calling convention.
//!
//! An export has a normal Seal body and an additional C-visible wrapper. The
//! record keeps the ABI and the source span; the function's signature lives in
//! its lowered body, so there is no second copy of the parameter types.

use seal_span::Span;
use seal_ty::Abi;

/// One `export "c" func` declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Export {
    /// The declared C ABI.
    pub abi: Abi,
    /// The span of the `export` modifier, for diagnostics.
    pub span: Span,
}
