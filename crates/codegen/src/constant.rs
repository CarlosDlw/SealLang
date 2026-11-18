//! Constant values and string literals.

use std::cell::{Cell, RefCell};

use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};
use seal_hash::FxHashMap;
use seal_symbol::{Interner, Symbol};

use crate::error::CodegenError;
use crate::types::TypeMapper;

use seal_mir::Constant;

/// Emits constants for one module.
///
/// String literals become private, unnamed constants. The value of a `*str` is
/// the fat pointer `{ data, length }`, so no hidden global state is needed at
/// run time and the length never depends on scanning for a terminator.
pub(crate) struct Constants<'a, 'ctx> {
    context: &'ctx Context,
    module: &'a Module<'ctx>,
    mapper: &'a TypeMapper<'ctx>,
    interner: &'a Interner,
    next_string: Cell<u32>,
    strings: RefCell<FxHashMap<String, (PointerValue<'ctx>, IntValue<'ctx>)>>,
}

impl<'a, 'ctx> Constants<'a, 'ctx> {
    /// Creates a constant emitter for `module`.
    pub(crate) fn new(
        context: &'ctx Context,
        module: &'a Module<'ctx>,
        mapper: &'a TypeMapper<'ctx>,
        interner: &'a Interner,
    ) -> Self {
        Self {
            context,
            module,
            mapper,
            interner,
            next_string: Cell::new(0),
            strings: RefCell::new(FxHashMap::default()),
        }
    }

    /// Returns a private, deduplicated global for `text` and its byte length.
    ///
    /// String literals and panic diagnostics share this cache, so repeated
    /// messages or file names occupy one global. The global is constant,
    /// unnamed-address, and has no trailing NUL; the length travels with the
    /// pointer.
    pub(crate) fn string_global(
        &self,
        text: &str,
    ) -> Result<(PointerValue<'ctx>, IntValue<'ctx>), CodegenError> {
        if let Some(value) = self.strings.borrow().get(text).copied() {
            return Ok(value);
        }
        let data = self.context.const_string(text.as_bytes(), false);
        let index = self.next_string.get();
        self.next_string.set(index + 1);
        let global = self
            .module
            .add_global(data.get_type(), None, &format!("seal.str.{index}"));
        global.set_initializer(&data);
        global.set_constant(true);
        global.set_unnamed_addr(true);
        let pointer = global.as_pointer_value();
        let length = self
            .mapper
            .usize_type()?
            .const_int(text.len() as u64, false);
        self.strings
            .borrow_mut()
            .insert(text.to_owned(), (pointer, length));
        Ok((pointer, length))
    }

    /// Returns the private global for an interned string and its byte length.
    ///
    /// Constant encoding names strings by symbol, so it resolves the text here
    /// instead of repeating the cache lookup at every call site.
    pub(crate) fn string_pointer(
        &self,
        symbol: Symbol,
    ) -> Result<(PointerValue<'ctx>, IntValue<'ctx>), CodegenError> {
        self.string_global(self.interner.resolve(symbol))
    }

    /// Returns the LLVM value of `constant`.
    pub(crate) fn emit(&self, constant: &Constant) -> Result<BasicValueEnum<'ctx>, CodegenError> {
        Ok(match constant {
            Constant::Int { value, ty } => {
                let int_ty = self.mapper.basic(*ty)?.into_int_type();
                if int_ty.get_bit_width() > 64 {
                    int_ty
                        .const_int_arbitrary_precision(&[*value as u64, (*value >> 64) as u64])
                        .into()
                } else {
                    int_ty.const_int(*value as u64, false).into()
                }
            }
            Constant::Float { value, ty } => self
                .mapper
                .basic(*ty)?
                .into_float_type()
                .const_float(*value)
                .into(),
            Constant::Bool(value) => self
                .context
                .bool_type()
                .const_int(u64::from(*value), false)
                .into(),
            Constant::Char(value) => self
                .context
                .i32_type()
                .const_int(u32::from(*value) as u64, false)
                .into(),
            Constant::Str { value, ty } => {
                let (data, length) = self.string_global(self.interner.resolve(*value))?;
                let fat = self
                    .mapper
                    .basic(*ty)?
                    .into_struct_type()
                    .const_named_struct(&[data.into(), length.into()]);
                fat.into()
            }
            Constant::Unit => self.mapper.empty_type().const_zero().into(),
        })
    }
}
