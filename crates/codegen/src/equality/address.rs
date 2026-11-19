//! Address arithmetic for walking a layout by byte offset.

use inkwell::values::{IntValue, PointerValue};
use seal_llvm_ffi::gep;

use crate::error::CodegenError;
use crate::function::FunctionLowerer;

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    /// Returns the address `offset` bytes past `pointer`.
    ///
    /// Every offset handed to this helper comes from the language layout and
    /// stays inside the value, so the GEP is inbounds and preserves provenance.
    pub(super) fn offset_by(
        &self,
        pointer: PointerValue<'ctx>,
        offset: IntValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        self.build(
            gep(
                self.builder,
                pointer,
                self.context.i8_type(),
                &[offset],
                true,
            ),
            "equality element pointer",
        )
    }
}
