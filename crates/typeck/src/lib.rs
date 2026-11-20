//! The Seal type checker.
//!
//! Type checking consumes the [`Hir`](seal_hir::Hir) after name resolution and
//! computes a [`TyId`](seal_ty::TyId) for every value expression, every
//! parameter, and every binding. It also builds a [`Signature`] for each
//! function so calls do not re-read the body.
//!
//! # Rules
//!
//! Checking is bidirectional: `check` gives an expression the type its context
//! requires, while `synth` computes the type of an expression that has no
//! expected type. Integer and float literals take the expected type when it is
//! compatible; otherwise they fall back to `i32` and `f64`. Suffixed literals
//! take their suffix, and an out-of-range literal is `E3009`.
//!
//! There is one implicit conversion: a thin pointer to a concrete value may
//! coerce to a trait-object pointer at a coercion site. Two distinct scalar
//! types never match (decision D12 of the memory model), and `!` is the only
//! type that fits any expected type because an expression of type `!` never
//! produces a value. Conditions must be `bool`, values stored in locals,
//! parameters, and return positions must be `Sized`, and assignments require a
//! mutable local.
//!
//! # Definite initialization
//!
//! The checker tracks which locals are initialized at every point. A `let`
//! without an initializer starts uninitialized; an assignment initializes it; an
//! `if` initializes a local only when both branches do, or when the branch that
//! does not initialize it diverges. A `while` restores the state from before the
//! loop, because the body may run zero times. Reading a possibly-uninitialized
//! local is `E3011`.
//!
//! # Determinism
//!
//! The checker emits diagnostics in source order. It never reads the host; the
//! pointer width used for `isize`/`usize` literal ranges comes from the
//! [`TargetSpec`](seal_target::TargetSpec) passed by the driver.

#![warn(missing_docs)]

mod checker;
mod results;

#[cfg(test)]
mod tests;

pub use results::{
    CaptureInfo, CaptureStorage, ClosureInfo, Coercion, IndexProtocol, IterationProtocol,
    MethodKind, PlaceProjection, PlaceRoot, ProtocolCall, ReceiverAdjust, ReceiverForm,
    ResolvedCall, ResolvedMethod, ResolvedPlace, Signature, TypeckResults,
};

use seal_diagnostics::DiagCtxt;
use seal_hir::Hir;
use seal_symbol::Interner;

/// Type-checks `hir`, reporting errors through `dcx`.
///
/// `pointer_width` is the width of `isize`/`usize` in bits; the driver takes it
/// from the target specification. The returned results are complete even when
/// errors were reported: an expression that failed to check has the `Error`
/// type, which suppresses cascading diagnostics.
pub fn check(hir: &Hir, dcx: &DiagCtxt, interner: &Interner, pointer_width: u8) -> TypeckResults {
    checker::Checker::new(hir, dcx, interner, pointer_width).run()
}
