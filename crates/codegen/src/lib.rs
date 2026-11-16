//! LLVM code generation for Seal.
//!
//! This crate lowers the typed MIR into LLVM IR and produces one artifact:
//! a relocatable object, target assembly, or a linked native executable.
//!
//! # Pipeline
//!
//! 1. Every MIR body is declared as an LLVM function first, so calls can target
//!    functions declared later.
//! 2. Each body is lowered with stack slots for its locals, explicit projections
//!    for fields and dereferences, and one LLVM block per MIR block.
//! 3. DWARF information is emitted alongside the IR when requested: compile unit,
//!    subroutine and aggregate types, parameters, bindings, and one location per
//!    statement and terminator.
//! 4. The module is verified, optimized at the requested level, and emitted.
//!    Executables are linked from a temporary object with the system C toolchain.
//!
//! # Layout
//!
//! Aggregates are LLVM structs in declaration order. Scalars use their size as
//! alignment, references use the target pointer width, `bool` is one byte, and
//! `()`/`!` are zero-sized structs. `*str` is the fat pointer `{ data, length }`.
//!
//! # Example
//!
//! ```ignore
//! let options = seal_codegen::CodegenOptions {
//!     emit: seal_codegen::Emit::Executable,
//!     output: "program".into(),
//!     opt_level: seal_session::OptLevel::O2,
//!     debug_info: true,
//!     crate_name: Some("program".to_owned()),
//! };
//! seal_codegen::emit(input, &options)?;
//! ```

#![warn(missing_docs)]

mod artifact;
mod backend;
mod constant;
mod debug;
mod entry;
mod error;
mod function;
mod names;
mod operand;
mod options;
mod place;
mod rvalue;
mod types;

pub use backend::{Artifact, CodegenInput, emit};
pub use error::{CodegenError, EntryPointError};
pub use options::{CodegenOptions, Emit};
