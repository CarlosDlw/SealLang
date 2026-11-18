//! Address arithmetic for walking a layout by byte offset.

use inkwell::values::{IntValue, PointerValue};

use crate::error::CodegenError;
use crate::function::FunctionLowerer;

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    /// Returns the address `offset` bytes past `pointer`.
    ///
    /// Inkwell exposes GEP only as an unsafe function, so the address is
    /// computed with integer arithmetic, exactly as field and array projection
    /// does. Every offset handed to this helper comes from the language layout,
    /// so it stays inside the value.
    pub(super) fn offset_by(
        &self,
        pointer: PointerValue<'ctx>,
        offset: IntValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let usize_type = self.mapper.usize_type()?;
        let base = self.build(
            self.builder
                .build_ptr_to_int(pointer, usize_type, "equality base"),
            "equality base address",
        )?;
        let address = self.build(
            self.builder.build_int_add(base, offset, "equality address"),
            "equality address",
        )?;
        self.build(
            self.builder
                .build_int_to_ptr(address, self.mapper.pointer(), "equality element"),
            "equality element pointer",
        )
    }
}
