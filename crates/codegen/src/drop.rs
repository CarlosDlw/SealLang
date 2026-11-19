//! Drop glue in code generation.
//!
//! Seal has no destructors yet, so every drop statement calls the shared no-op
//! entry, and the vtable header uses it as its drop slot. The statement shape
//! and the call are already what the MIR carries; the phase that introduces
//! destructors replaces the body with the real per-type glue, without changing
//! the call sites.

use inkwell::context::Context;
use inkwell::module::{Linkage, Module};
use inkwell::values::FunctionValue;

use crate::error::CodegenError;
use crate::names;

/// Returns the shared drop-glue entry, declaring it on first use.
///
/// The function is internal and takes the erased data pointer, matching the
/// documented slot signature of the vtable header.
pub(crate) fn noop_drop<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
) -> Result<FunctionValue<'ctx>, CodegenError> {
    if let Some(function) = module.get_function(names::DROP) {
        return Ok(function);
    }
    let pointer = context.ptr_type(inkwell::AddressSpace::default());
    let function_type = context.void_type().fn_type(&[pointer.into()], false);
    let function = module.add_function(names::DROP, function_type, Some(Linkage::Internal));
    let entry = context.append_basic_block(function, "entry");
    let builder = context.create_builder();
    builder.position_at_end(entry);
    builder.build_return(None).map_err(|error| {
        CodegenError::internal(format!("cannot build the drop glue function: {error:?}"))
    })?;
    Ok(function)
}
