//! Bridge for the LLVM 19+ debug-record API.
//!
//! LLVM 19 replaced debug intrinsics with *debug records*. The C API function
//! that appends a variable declaration now returns a debug record handle, not an
//! instruction. inkwell 0.11 still wraps that return value in an
//! `InstructionValue`, which trips an assertion in debug builds and produces an
//! invalid handle in release builds. The record itself is attached to the block
//! before the wrapper is constructed, so the only correct operation is to call
//! the C API and discard the returned record handle.
//!
//! This crate exists solely for that call. It is the only place in the compiler
//! that uses `unsafe`, which keeps `unsafe_code = "forbid"` meaningful for every
//! other crate.

#![warn(missing_docs)]

use inkwell::basic_block::BasicBlock;
use inkwell::debug_info::{DIExpression, DILocalVariable, DILocation, DebugInfoBuilder};
use inkwell::values::{AsValueRef, PointerValue};
use llvm_sys::debuginfo::LLVMDIBuilderInsertDeclareRecordAtEnd;

/// Declares `storage` as the storage of `variable` at the end of `block`.
///
/// The record is owned by `block`; the returned handle is not needed and is
/// discarded. Callers must pass handles from the same LLVM context.
pub fn declare_at_end(
    builder: &DebugInfoBuilder<'_>,
    storage: PointerValue<'_>,
    variable: DILocalVariable<'_>,
    expression: DIExpression<'_>,
    location: DILocation<'_>,
    block: BasicBlock<'_>,
) {
    // SAFETY: all handles come from the same live LLVM context owned by the
    // caller. The C API only appends a debug record to `block` and returns a
    // borrowed record handle. Every pointer is non-null because inkwell's
    // metadata and value handles never wrap null pointers.
    unsafe {
        LLVMDIBuilderInsertDeclareRecordAtEnd(
            builder.as_mut_ptr(),
            storage.as_value_ref(),
            variable.as_mut_ptr(),
            expression.as_mut_ptr(),
            location.as_mut_ptr(),
            block.as_mut_ptr(),
        );
    }
}
