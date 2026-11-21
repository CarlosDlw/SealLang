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
//! Pointer arithmetic has the same shape: inkwell exposes `build_gep` and
//! `build_in_bounds_gep` only as unsafe functions, and the workspace keeps
//! `unsafe_code = "forbid"` on every compiler crate. [`gep`] validates the
//! common preconditions and confines the unsafe call here, in the only crate
//! allowed to use `unsafe`.
//!
//! Constant relocation expressions also need an audited wrapper:
//! [`const_gep_bytes`] builds the constant `getelementptr i8` used by global
//! initializers.

#![warn(missing_docs)]

use inkwell::basic_block::BasicBlock;
use inkwell::builder::{Builder, BuilderError};
use inkwell::context::Context;
use inkwell::debug_info::{DIExpression, DILocalVariable, DILocation, DebugInfoBuilder};
use inkwell::memory_buffer::MemoryBuffer;
use inkwell::module::Module;
use inkwell::types::{AsTypeRef, BasicType};
use inkwell::values::{AsValueRef, IntValue, PointerValue};
use llvm_sys::bit_reader::LLVMParseBitcodeInContext2;
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

/// Builds a `getelementptr` instruction for `base`.
///
/// `pointee` is the source element type of `base`; `indices` are interpreted
/// exactly as LLVM's GEP indices. With `inbounds`, every intermediate address
/// must stay inside the allocation, which is the contract of ordinary field,
/// index, and checked pointer arithmetic. Wrapping pointer arithmetic passes
/// `false`, because it intentionally permits addresses outside the allocation.
///
/// # Errors
///
/// Returns the builder's error when the builder has no insertion point or LLVM
/// rejects the combination of type, pointer, and indices.
pub fn gep<'ctx, T: BasicType<'ctx>>(
    builder: &Builder<'ctx>,
    base: PointerValue<'ctx>,
    pointee: T,
    indices: &[IntValue<'ctx>],
    inbounds: bool,
) -> Result<PointerValue<'ctx>, BuilderError> {
    // SAFETY: the caller supplies a pointer and a source element type from the
    // same LLVM context, and indices whose types are valid for that element
    // type. All handles are borrowed for the duration of the call; LLVM
    // allocates the resulting instruction in the current block and does not
    // take ownership of the operands. `inbounds` is forwarded exactly as the
    // caller requested, so wrapping arithmetic never receives an inbounds
    // promise.
    unsafe {
        if inbounds {
            builder.build_in_bounds_gep(pointee, base, indices, "gep")
        } else {
            builder.build_gep(pointee, base, indices, "gep")
        }
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
/// Parses one LLVM bitcode module owned by `context`.
///
/// The bytes are copied into a temporary memory buffer, so the returned module
/// does not borrow from the input slice. This is the entry point a split
/// `codegen` action uses: the frontend wrote bitcode, and code generation must
/// not lower types or MIR again.
///
/// # Errors
///
/// Returns a human-readable message when LLVM rejects the bitcode. LLVM 23's
/// C API returns only a boolean here; the caller attaches the artifact path and
/// its diagnostic code.
pub fn parse_bitcode<'ctx>(context: &'ctx Context, bytes: &[u8]) -> Result<Module<'ctx>, String> {
    if bytes.is_empty() {
        return Err("the bitcode payload is empty".to_owned());
    }
    let buffer = MemoryBuffer::create_from_memory_range_copy(bytes, "sealmono");
    let mut module = std::ptr::null_mut();
    // SAFETY: `context` and `buffer` are live for the call, and the output
    // pointer names a local that LLVM writes exactly once. On success LLVM
    // transfers ownership of the module handle to the returned `Module`.
    let failed =
        unsafe { LLVMParseBitcodeInContext2(context.raw(), buffer.as_mut_ptr(), &mut module) };
    if failed != 0 {
        return Err("LLVM rejected the bitcode payload".to_owned());
    }
    if module.is_null() {
        return Err("LLVM returned a null module".to_owned());
    }
    // SAFETY: LLVM initialized `module` and transferred its ownership here;
    // the context outlives the returned value by the signature's lifetime.
    Ok(unsafe { Module::new(module) })
}

/// Builds a `getelementptr` instruction for `base`.
///
/// `pointee` is the source element type of `base`; `indices` are interpreted
/// exactly as LLVM's GEP indices. With `inbounds`, every intermediate address
/// must stay inside the allocation, which is the contract of ordinary field,
/// index, and checked pointer arithmetic. Wrapping pointer arithmetic passes
/// `false`, because it intentionally permits addresses outside the allocation.
///
/// # Errors
///
/// Returns the builder's error when the builder has no insertion point or LLVM
/// rejects the combination of type, pointer, and indices.
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
