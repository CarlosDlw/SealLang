//! Constant evaluation for Seal.
//!
//! The evaluator runs over MIR after type checking. It produces a
//! [`ConstProgram`] holding every evaluated value, the allocations that
//! references point into, the static data of each static, and the dependency
//! edges used to detect cycles. Later phases read that program instead of
//! re-evaluating source.

#![warn(missing_docs)]

mod error;
mod eval;
mod limits;
mod ops;
mod place;
mod program;
mod value;

pub use error::{EvalError, EvalErrorKind};
pub use eval::{evaluate, evaluate_with_limits};
pub use limits::EvalLimits;
pub use place::{PathStep, ResolvedPlace};
pub use program::ConstProgram;
pub use value::{
    AllocId, AllocSource, Allocations, ConstAllocation, ConstPointer, ConstValue, Dependency,
    DependencyKind, StaticData, TraitObjectKey,
};
