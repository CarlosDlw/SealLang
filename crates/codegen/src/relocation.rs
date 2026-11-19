//! Pointer slots inside constant data.
//!
//! A constant value is encoded as its exact memory image, so every address it
//! contains becomes a pointer-sized slot at a known byte offset. The encoder
//! records those slots symbolically; this module turns them into LLVM constant
//! pointers once every global exists.
//!
//! Resolving late is what makes cycles work: a static may refer to a static
//! declared after it, and a constant allocation may refer to itself, because the
//! address of a declared global is valid before its initializer is set.

use inkwell::context::Context;
use inkwell::values::PointerValue;
use seal_const_eval::AllocId;
use seal_mono::VtableKey;
use seal_symbol::Symbol;
use seal_ty::{TraitRef, TyId};

use crate::constant::Constants;
use crate::error::CodegenError;
use crate::global::Globals;
use crate::vtable::Vtables;

/// The symbolic target of one pointer slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PointerTarget {
    /// The address of an allocation, advanced by a byte offset.
    Allocation {
        /// The referenced allocation.
        id: AllocId,
        /// Byte offset inside the allocation.
        offset: u64,
    },
    /// The data pointer of an interned string, advanced by a byte offset.
    String {
        /// The interned string.
        symbol: Symbol,
        /// Byte offset inside the string data.
        offset: u64,
    },
    /// The vtable of a trait object.
    Vtable {
        /// The concrete erased type.
        concrete: TyId,
        /// The principal trait reference that selects the table.
        trait_ref: TraitRef,
    },
}

/// Resolves symbolic pointer slots against the emitted module.
pub(crate) struct Pointers<'a, 'ctx> {
    context: &'ctx Context,
    strings: &'a Constants<'a, 'ctx>,
    vtables: &'a Vtables<'ctx>,
    globals: &'a Globals<'ctx>,
}

impl<'a, 'ctx> Pointers<'a, 'ctx> {
    /// Creates a resolver for one module.
    pub(crate) fn new(
        context: &'ctx Context,
        strings: &'a Constants<'a, 'ctx>,
        vtables: &'a Vtables<'ctx>,
        globals: &'a Globals<'ctx>,
    ) -> Self {
        Self {
            context,
            strings,
            vtables,
            globals,
        }
    }

    /// Returns the constant pointer for `target`.
    ///
    /// A missing allocation or vtable is an internal invariant violation: the
    /// monomorphizer validates every global before code generation, so a gap
    /// here means the collector and the backend disagree.
    pub(crate) fn resolve(
        &self,
        target: PointerTarget,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        Ok(match target {
            PointerTarget::Allocation { id, offset } => {
                let global = self.globals.allocation(id)?;
                seal_llvm_ffi::const_gep_bytes(self.context, global.as_pointer_value(), offset)
            }
            PointerTarget::String { symbol, offset } => seal_llvm_ffi::const_gep_bytes(
                self.context,
                self.strings.string_pointer(symbol)?.0,
                offset,
            ),
            PointerTarget::Vtable {
                concrete,
                trait_ref,
            } => self
                .vtables
                .global_for_key(VtableKey {
                    concrete,
                    trait_ref,
                })?
                .as_pointer_value(),
        })
    }
}
