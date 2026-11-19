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

mod adt;
mod cast;
mod ctxt;
mod display;
mod drop;
mod generics;
mod id;
mod infer;
mod init;
mod kind;
mod substs;
mod template;

pub use adt::{AdtDef, AdtKind, FieldDef, VariantDef};
pub use cast::NumericCastKind;
pub use ctxt::{ImplHead, TyCtxt};
pub use display::TyDisplay;
pub use generics::{
    Bound, GenericParamDef, GenericSignature, Generics, Instance, ParamEnv, ParamTy, TraitDef,
    TraitImplDef, TraitMethodDef, TraitMethodReceiver, TraitRef,
};
pub use id::{AdtId, InferTyId, ParamTyId, SubstsId, TupleId, TyId, VariantIdx};
pub use infer::{InferCtxt, UnifyError};
pub use init::{InitPaths, InitState, PathElem, place_type};
pub use kind::{FloatTy, IntTy, RawPtrKind, RefKind, TyKind, UintTy};
pub use template::{TemplateMatch, TemplateUnifier, match_template, unify_templates};
