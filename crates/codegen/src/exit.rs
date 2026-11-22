//! Emitting the `exit(code)` built-in.
//!
//! A normal target links the runtime's `seal_exit`, which calls the platform
//! process-exit primitive without flushing or unwinding. A freestanding target
//! has no runtime archive, so it calls the `exit` implementation supplied by
//! the freestanding link instead. Both functions never return, which lets the
//! caller terminate the LLVM block with `unreachable`.

use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::values::FunctionValue;
use seal_mir::Operand;

use crate::error::CodegenError;
use crate::function::FunctionLowerer;

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    /// Calls the process-exit primitive with `code` and terminates the block.
    pub(crate) fn exit_with(&self, code: &Operand) -> Result<(), CodegenError> {
        let code = self.load_operand(code)?.into_int_value();
        let symbol = if self.runtime_available {
            "seal_exit"
        } else {
            "exit"
        };
        let function = self.exit_function(symbol)?;
        self.build(
            self.builder.build_call(function, &[code.into()], "exit"),
            "exit call",
        )?;
        self.build(self.builder.build_unreachable(), "exit terminator")?;
        Ok(())
    }

    /// Declares the one-argument, non-returning process-exit function.
    fn exit_function(&self, symbol: &str) -> Result<FunctionValue<'ctx>, CodegenError> {
        if let Some(function) = self.module.get_function(symbol) {
            return Ok(function);
        }
        let i32_type = self.context.i32_type();
        let function_type = self.context.void_type().fn_type(&[i32_type.into()], false);
        let function = self.module.add_function(symbol, function_type, None);
        for name in ["noreturn", "nounwind", "cold"] {
            let attribute = self
                .context
                .create_enum_attribute(Attribute::get_named_enum_kind_id(name), 0);
            function.add_attribute(AttributeLoc::Function, attribute);
        }
        Ok(function)
    }
}
