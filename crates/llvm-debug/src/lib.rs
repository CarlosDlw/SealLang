//! Bridge for the LLVM APIs inkwell exposes unsafely or not at all.
//!
//! LLVM 19 replaced debug intrinsics with *debug records*. The C API function
//! that appends a variable declaration now returns a debug record handle, not an
//! instruction. inkwell 0.11 still wraps that return value in an
//! `InstructionValue`, which trips an assertion in debug builds and produces an
//! invalid handle in release builds. The record itself is attached to the block
//! before the wrapper is constructed, so the only correct operation is to call
//! the C API and discard the returned record handle.
//!
//! Constant relocation expressions need the same treatment: inkwell exposes
//! `PointerValue::const_gep` only as an unsafe function, and the workspace keeps
//! `unsafe_code = "forbid"` on every compiler crate. Both calls live here, in
//! the only crate allowed to use `unsafe`.

#![warn(missing_docs)]

use inkwell::basic_block::BasicBlock;
use inkwell::context::Context;
use inkwell::debug_info::{DIExpression, DILocalVariable, DILocation, DebugInfoBuilder};
use inkwell::types::AsTypeRef;
use inkwell::values::{AsValueRef, PointerValue};
use llvm_sys::core::LLVMConstGEP2;
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

/// Returns `pointer` advanced by `byte_offset` bytes as a constant expression.
///
/// The result is a `getelementptr i8, ptr %pointer, i64 %byte_offset` constant,
/// which every global initializer and every debug record can use. A zero offset
/// returns the pointer unchanged instead of emitting a no-op constant.
///
/// # Panics
///
/// Panics when LLVM refuses to build the constant expression. That cannot
/// happen for a pointer and an integer index in the same live context, so a
/// failure means a handle from another context was passed.
pub fn const_gep_bytes<'ctx>(
    context: &'ctx Context,
    pointer: PointerValue<'ctx>,
    byte_offset: u64,
) -> PointerValue<'ctx> {
    if byte_offset == 0 {
        return pointer;
    }
    let index = context.i64_type().const_int(byte_offset, false);
    // SAFETY: all handles come from the same live LLVM context. The source
    // element type is `i8`, so the single index is a byte offset, and
    // `LLVMConstGEP2` only interns a constant expression without taking
    // ownership of its operands. LLVM allocates a new constant node, so the
    // returned pointer is never null.
    unsafe {
        let mut indices = [index.as_value_ref()];
        let value = LLVMConstGEP2(
            context.i8_type().as_type_ref(),
            pointer.as_value_ref(),
            indices.as_mut_ptr(),
            1,
        );
        assert!(!value.is_null(), "LLVM returned a null constant GEP");
        PointerValue::new(value)
    }
}
