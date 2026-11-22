//! Emission of runtime panics with source locations.
//!
//! Panic calls target the C runtime's `seal_panic` directly. The declaration is
//! marked `noreturn`, `nounwind`, and `cold` so LLVM can omit a fallthrough
//! path, avoid unwind-table assumptions, and keep the failure path out of the
//! hot code layout. The call is followed by an `unreachable` terminator because
//! the runtime exists on every path.

use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::types::BasicMetadataTypeEnum;
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue};
use seal_mir::Operand;
use seal_span::Span;

use crate::error::CodegenError;
use crate::function::FunctionLowerer;

/// Symbol exported by the runtime archive.
const PANIC_SYMBOL: &str = "seal_panic";

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    /// Reports a panic at `span` and terminates the current block.
    ///
    /// The caller must position the builder in the block that should not
    /// continue. On success the builder is left on a terminated block.
    pub(crate) fn panic_at(&self, span: Span, message: &str) -> Result<(), CodegenError> {
        if self.trap_panics {
            // A freestanding target has no runtime to format or report a
            // panic through. An `unreachable` terminator is a hardware trap on
            // every supported target and keeps the artifact runtime-free.
            let _ = (span, message);
            self.builder.build_unreachable().map_err(|error| {
                CodegenError::internal(format!("cannot emit a panic trap: {error:?}"))
            })?;
            return Ok(());
        }
        let (message_data, message_length) = self.constants.string_global(message)?;
        self.panic_with(span, message_data, message_length)
    }

    /// Reports a computed `*str` message at `span` and terminates the block.
    ///
    /// The runtime receives the fat pointer's data and length exactly like a
    /// literal message, so both panic forms share one reporting contract.
    pub(crate) fn panic_value_at(&self, span: Span, message: &Operand) -> Result<(), CodegenError> {
        if self.trap_panics {
            let _ = span;
            self.builder.build_unreachable().map_err(|error| {
                CodegenError::internal(format!("cannot emit a panic trap: {error:?}"))
            })?;
            return Ok(());
        }
        let value = self.load_operand(message)?;
        let BasicValueEnum::StructValue(fat) = value else {
            return Err(CodegenError::internal(
                "a panic message is not a `*str` fat pointer",
            ));
        };
        let data = self
            .build(
                self.builder
                    .build_extract_value(fat, 0, "panic.message.data"),
                "panic message data",
            )?
            .into_pointer_value();
        let length = self
            .build(
                self.builder
                    .build_extract_value(fat, 1, "panic.message.length"),
                "panic message length",
            )?
            .into_int_value();
        self.panic_with(span, data, length)
    }

    /// Calls the runtime panic reporter and leaves the block terminated.
    fn panic_with(
        &self,
        span: Span,
        message_data: inkwell::values::PointerValue<'ctx>,
        message_length: inkwell::values::IntValue<'ctx>,
    ) -> Result<(), CodegenError> {
        let (file, line, column) = self.source_location(span);
        let (file_data, file_length) = self.constants.string_global(&file)?;
        let function = self.panic_function()?;
        let line = self.context.i32_type().const_int(u64::from(line), false);
        let column = self.context.i32_type().const_int(u64::from(column), false);
        let arguments = [
            BasicMetadataValueEnum::PointerValue(message_data),
            BasicMetadataValueEnum::IntValue(message_length),
            BasicMetadataValueEnum::PointerValue(file_data),
            BasicMetadataValueEnum::IntValue(file_length),
            BasicMetadataValueEnum::IntValue(line),
            BasicMetadataValueEnum::IntValue(column),
        ];
        self.build(
            self.builder.build_call(function, &arguments, "panic"),
            "panic call",
        )?;
        self.build(self.builder.build_unreachable(), "panic terminator")?;
        Ok(())
    }

    fn panic_function(&self) -> Result<FunctionValue<'ctx>, CodegenError> {
        if let Some(function) = self.module.get_function(PANIC_SYMBOL) {
            return Ok(function);
        }
        let pointer = self.mapper.pointer();
        let usize_type = self.mapper.usize_type()?;
        let i32_type = self.context.i32_type();
        let parameters = [
            BasicMetadataTypeEnum::PointerType(pointer),
            BasicMetadataTypeEnum::IntType(usize_type),
            BasicMetadataTypeEnum::PointerType(pointer),
            BasicMetadataTypeEnum::IntType(usize_type),
            BasicMetadataTypeEnum::IntType(i32_type),
            BasicMetadataTypeEnum::IntType(i32_type),
        ];
        let function_type = self.context.void_type().fn_type(&parameters, false);
        let function = self.module.add_function(PANIC_SYMBOL, function_type, None);
        for name in ["noreturn", "nounwind", "cold"] {
            let attribute = self
                .context
                .create_enum_attribute(Attribute::get_named_enum_kind_id(name), 0);
            function.add_attribute(AttributeLoc::Function, attribute);
        }
        Ok(function)
    }

    fn source_location(&self, span: Span) -> (String, u32, u32) {
        let Some(file_id) = self.source_map.file_of(span) else {
            return ("<unknown>".to_owned(), 0, 0);
        };
        let file = self
            .source_map
            .display_name(file_id)
            .unwrap_or_else(|| "<unknown>".to_owned());
        let position = self
            .source_map
            .file(file_id)
            .and_then(|source| source.line_col(span.lo()).ok())
            .map(|position| (position.line, position.column))
            .unwrap_or((0, 0));
        (file, position.0, position.1)
    }
}
