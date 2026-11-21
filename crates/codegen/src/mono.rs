//! Code generation from one `.sealmono` unit.
//!
//! The frontend already lowered types and MIR and wrote the unit's bitcode.
//! This module parses that bitcode, runs the unit's optimization pipeline, and
//! asks LLVM for a native object; it never lowers a type, a body, or a generic
//! instance again. Keeping optimization here is what makes the expensive passes
//! run in parallel across `codegen-cgu` actions.

use std::path::Path;

use inkwell::OptimizationLevel;
use inkwell::context::Context;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetTriple,
};
use seal_session::OptLevel;

use crate::error::CodegenError;

/// Emits one native object from a frontend bitcode payload.
///
/// # Errors
///
/// Returns [`CodegenError`] when the target is unsupported, the bitcode is
/// invalid, LLVM cannot generate the object, or the destination cannot be
/// written.
pub fn emit_object_from_bitcode(
    bitcode: &[u8],
    output: &Path,
    target_triple: &str,
    opt_level: u8,
) -> Result<(), CodegenError> {
    Target::initialize_all(&InitializationConfig::default());
    let triple = TargetTriple::create(target_triple);
    let target = Target::from_triple(&triple).map_err(|error| {
        CodegenError::unsupported_target(
            target_triple,
            format!("LLVM does not support this target: {error}"),
        )
    })?;
    let level = match opt_level {
        0 => OptimizationLevel::None,
        1 => OptimizationLevel::Less,
        2 => OptimizationLevel::Default,
        _ => OptimizationLevel::Aggressive,
    };
    let pipeline = match opt_level {
        0 => OptLevel::O0,
        1 => OptLevel::O1,
        2 => OptLevel::O2,
        _ => OptLevel::O3,
    };
    let machine = target
        .create_target_machine(
            &triple,
            "generic",
            "",
            level,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .ok_or_else(|| {
            CodegenError::unsupported_target(
                target_triple,
                "LLVM could not create a target machine",
            )
        })?;
    let context = Context::create();
    let module = seal_llvm_ffi::parse_bitcode(&context, bitcode).map_err(|error| {
        CodegenError::artifact(
            output,
            format!("cannot parse the frontend bitcode: {error}"),
        )
    })?;
    crate::optimize::run(&module, &machine, pipeline)?;
    let buffer = machine
        .write_to_memory_buffer(&module, FileType::Object)
        .map_err(|error| {
            CodegenError::artifact(
                output,
                format!("LLVM could not generate the object: {error}"),
            )
        })?;
    seal_fs::write_atomic(output, buffer.as_slice()).map_err(|error| {
        CodegenError::artifact(output, format!("cannot write the object: {error}"))
    })
}
