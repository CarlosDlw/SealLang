module alloc;

use core::result::Result;

// Allocates one block for a validated layout.
//
// The returned pointer is non-null, aligned to `layout.align()`, and valid for
// `layout.size()` bytes. The bytes are uninitialized. The caller owns the block
// and must release it exactly once with `dealloc` and the same layout.
//
// A zero-size layout returns the runtime's non-null dangling pointer. It is a
// successful result, but it does not name storage and must never be
// dereferenced; passing it to `dealloc` with the same layout is a no-op.
//
// # Safety
//
// The caller must keep the returned block live until deallocation and must
// never use a pointer from a zero-size layout as an addressable byte.
public unsafe func alloc(layout: Layout) -> Result<*raw mut u8, AllocError> {
  let pointer = unsafe { seal_alloc(layout.size(), layout.align()) };
  if pointer.is_null() {
    Result::Err(AllocError {})
  } else {
    Result::Ok(pointer)
  }
}

// Allocates one block for a validated layout with every byte zeroed.
//
// The returned pointer has the same ownership, alignment, lifetime, and
// zero-size rules as `alloc`. The caller owns the block and must release it
// exactly once with `dealloc` and the same layout.
//
// # Safety
//
// The caller must keep the returned block live until deallocation and must
// never use a pointer from a zero-size layout as an addressable byte.
public unsafe func alloc_zeroed(layout: Layout) -> Result<*raw mut u8, AllocError> {
  let pointer = unsafe { seal_alloc_zeroed(layout.size(), layout.align()) };
  if pointer.is_null() {
    Result::Err(AllocError {})
  } else {
    Result::Ok(pointer)
  }
}

// Releases a block returned by `alloc` or `alloc_zeroed`.
//
// A null pointer and a zero-size layout are no-ops, matching the runtime. For
// every real block, `layout` must be the exact layout that created it. The
// pointer is invalid after this call, and releasing the same block twice is
// undefined behavior.
//
// # Safety
//
// The caller must pass a pointer returned by this module, not a pointer
// obtained from C `malloc`, and must pass the exact layout that created the
// block. The block must not be used after this call and must not be released
// again.
public unsafe func dealloc(pointer: *raw mut u8, layout: Layout) {
  if pointer.is_null() || layout.size() == 0 {
    return;
  }
  unsafe { seal_dealloc(pointer, layout.size(), layout.align()); }
}
