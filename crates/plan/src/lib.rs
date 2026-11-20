//! The versioned compile plan exchanged between the project layer and `sealc`.
//!
//! A plan is data, not code: it names the target, profile, packages, files,
//! exposed modules, dependency aliases, and the requested output. The driver
//! validates the plan before loading any source, so an invalid plan is a clear
//! `E8001`/`E8010` diagnostic instead of a panic or a half-compiled program.

#![warn(missing_docs)]

mod compile;
mod error;
mod serialize;

pub use compile::{
    BuildMode, CfgOptions, CompilePlan, FORMAT_VERSION, PlanDependency, PlanFile, PlanMetadata,
    PlanPackage, PlanProfile,
};
pub use error::{PlanError, PlanResult};
