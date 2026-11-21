//! LLVM code generation for Seal.
//!
//! This crate lowers the typed MIR into LLVM IR and produces one artifact:
//! a relocatable object, target assembly, or a linked native executable.
//!
//! # Pipeline
//!
//! 1. Every monomorphized instance's body is declared as an LLVM function first,
//!    so calls can target functions declared later. A generic instance appends a
//!    stable fingerprint of its type arguments to its symbol. One internal drop
//!    glue function per concrete type is declared and filled next, then vtable
//!    globals, because their first slot names the concrete type's glue and the
//!    remaining slots name method functions.
//! 2. Every reachable static and const allocation is declared, then initialized
//!    from its evaluated memory image. Declaring before initializing lets a
//!    global name one that appears later in the program. A `const` read by value
//!    is copied from a private constant global created on first use.
//! 3. Each body is lowered with stack slots for its locals, explicit projections
//!    for fields and dereferences, and one LLVM block per MIR block. Generic
//!    aggregates get one LLVM struct per instantiation. A static place is the
//!    address of its global.
//! 4. DWARF information is emitted alongside the IR when requested: compile unit,
//!    subroutine and aggregate types, parameters, bindings, statics, and one
//!    location per statement and terminator.
//! 5. The module is verified, optimized at the requested level, and emitted.
//!    Executables are linked from a temporary object and the embedded C runtime
//!    archive with the system C toolchain.
//!
//! Runtime failures (division by zero and signed division overflow) call
//! `seal_panic`, which writes a structured diagnostic to standard error and
//! exits with status 101. The runtime core never allocates or uses stdio; the
//! heap layer is a separate component, linked only when an allocation symbol
//! is referenced.
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
//!     runtime_archive: None,
//!     libraries: Vec::new(),
//!     search_paths: Vec::new(),
//!     link_args: Vec::new(),
//!     entry: None,
//!     cgu_count: 1,
//! };
//! seal_codegen::emit(input, &options)?;
//! ```

#![warn(missing_docs)]

mod artifact;
mod backend;
mod c_abi;
mod callback;
mod cast;
mod closure;
mod const_image;
mod const_value;
mod constant;
mod debug;
mod drop_glue;
mod entry;
mod equality;
mod error;
mod extern_decl;
mod function;
mod global;
mod intrinsic;
mod linker;
mod mono;
mod names;
mod object;
mod operand;
mod options;
mod panic;
mod place;
mod raw_intrinsic;
mod relocation;
mod runtime;
mod rvalue;
mod trait_object;
mod types;
mod vtable;

#[cfg(test)]
mod tests;

pub use backend::{Artifact, CodegenInput, emit};
pub use error::{CodegenError, EntryPointError};
pub use mono::emit_object_from_bitcode;
pub use object::{ObjectLink, link_objects};
pub use options::{CodegenOptions, Emit};

/// Returns the runtime archive embedded in this compiler build.
///
/// The bytes are the host archive used by default when linking an executable.
/// They can be written to disk for manual links of object files produced with
/// [`Emit::Object`].
pub fn runtime_archive() -> &'static [u8] {
    runtime::ARCHIVE
}
