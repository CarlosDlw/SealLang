//! LLVM optimization shared by whole-module and per-CGU code generation.

use inkwell::module::Module;
use inkwell::passes::PassBuilderOptions;
use inkwell::targets::TargetMachine;

use crate::error::CodegenError;
use seal_session::OptLevel;

/// Returns the LLVM pass pipeline for `level`.
///
/// `O0` is the identity pipeline: the backend leaves the module untouched so
/// debug builds keep source-level structure and compile quickly.
pub(crate) const fn pipeline(level: OptLevel) -> &'static str {
    match level {
        OptLevel::O0 => "default<O0>",
        OptLevel::O1 => "default<O1>",
        OptLevel::O2 => "default<O2>",
        OptLevel::O3 => "default<O3>",
    }
}

/// Runs the pipeline for `level` over `module`.
///
/// # Errors
///
/// Returns [`CodegenError`] when LLVM rejects the pipeline. The error is
/// internal: the pipeline is built from a closed set of levels, so a failure
/// means the LLVM installation cannot run the passes it advertises.
pub(crate) fn run(
    module: &Module<'_>,
    machine: &TargetMachine,
    level: OptLevel,
) -> Result<(), CodegenError> {
    if level == OptLevel::O0 {
        return Ok(());
    }
    module
        .run_passes(pipeline(level), machine, PassBuilderOptions::create())
        .map_err(|error| CodegenError::internal(format!("LLVM optimization failed: {error}")))
}
