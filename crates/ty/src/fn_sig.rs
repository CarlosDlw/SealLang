//! Interned C function signatures.
//!
//! A function signature describes the pointee of `*raw extern "c" func(...)`.
//! It is a type-level description, not a value: only a pointer to it has a
//! representation. The ABI, the ordered parameter types, the return type, and
//! variadicity are all part of the signature's identity.

use crate::Abi;
use crate::TyId;

/// One C function signature.
///
/// The definition is stored in the [`TyCtxt`](crate::TyCtxt) arena and shared
/// by every occurrence of the same signature.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FnSigDef {
    /// The calling convention written after `extern`.
    pub abi: Abi,
    /// Fixed parameter types in declaration order.
    pub params: Vec<TyId>,
    /// The return type, or `()` when the source omits it.
    pub ret: TyId,
    /// Whether the signature ends with `...`.
    pub variadic: bool,
}
