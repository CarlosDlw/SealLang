//! The Seal type system: interned semantic types and unification.
//!
//! [`TyCtxt`] interns every [`TyKind`], so two types are equal exactly when
//! their [`TyId`]s are equal. Types written in the source are syntax; the types
//! in this crate are their meaning, independent of where they were written.
//!
//! Type inference happens through [`InferCtxt`]: it creates variables, binds
//! them during [`InferCtxt::unify`], and rejects a binding that would make a
//! variable contain itself. Unification never performs an implicit conversion;
//! equal kinds are the only types that unify.
//!
//! The crate also encodes the memory model documented in
//! `docs/memory-model.md`: `*T` is a shared, non-null reference, `*mut T` is its
//! exclusive counterpart, `str` is unsized, and `Copy`, `Sized`, and `Drop` are
//! queries derived from the type rather than flags stored next to it.

#![warn(missing_docs)]

mod abi;
mod adt;
mod cast;
mod copy;
mod ctxt;
mod display;
mod drop;
mod ffi;
mod fn_ptr;
mod fn_sig;
mod generics;
mod id;
mod infer;
mod init;
mod kind;
mod substs;
mod template;

pub use abi::{Abi, CallConv};
pub use adt::{AdtDef, AdtKind, AdtRepr, FieldDef, VariantDef};
pub use cast::{CastKind, NumericCastKind};
pub use ctxt::{ImplHead, TyCtxt};
pub use display::TyDisplay;
pub use ffi::{FfiPosition, NotFfiSafe, safety};
pub use fn_ptr::{ClosureKind, FnPtrDef};
pub use fn_sig::FnSigDef;
pub use generics::{
    Bound, GenericParamDef, GenericSignature, Generics, Instance, ParamEnv, ParamTy, TraitDef,
    TraitImplDef, TraitMethodDef, TraitMethodReceiver, TraitRef,
};
pub use id::{AdtId, FnPtrId, FnSigId, InferTyId, ParamTyId, SubstsId, TupleId, TyId, VariantIdx};
pub use infer::{InferCtxt, UnifyError};
pub use init::{InitPaths, InitState, PathElem, place_type};
pub use kind::{FloatTy, IntTy, RawPtrKind, RefKind, TyKind, UintTy};
pub use seal_target::CAbiProperties;
pub use template::{TemplateMatch, TemplateUnifier, match_template, unify_templates};
