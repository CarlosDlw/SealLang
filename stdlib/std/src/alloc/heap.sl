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

// Resizes a block returned by `alloc` or `realloc`.
//
// `pointer` and `old_layout` must describe the current live block, and
// `new_size` must be greater than zero. On success the returned pointer is
// aligned to `old_layout.align()`, the first `min(old_size, new_size)` bytes are
// preserved, and any grown tail is uninitialized. On failure the old block is
// still live, unchanged, and owned by the caller.
//
// A zero `new_size` is rejected with `AllocError` without releasing the block;
// use `dealloc` to release it. A null pointer is outside this contract; use
// `alloc` for a first allocation.
//
// # Safety
//
// The caller must pass a pointer returned by `alloc` or `realloc`, the exact
// layout that created it, and a non-zero `new_size`. The block must not be used
// through the old pointer after a successful resize.
public unsafe func realloc(
  pointer: *raw mut u8,
  old_layout: Layout,
  new_size: usize,
) -> Result<*raw mut u8, AllocError> {
  if new_size == 0 {
    return Result::Err(AllocError {});
  }

  let resized = unsafe {
    seal_realloc(pointer, old_layout.size(), old_layout.align(), new_size)
  };
  if resized.is_null() {
    Result::Err(AllocError {})
  } else {
    Result::Ok(resized)
  }
}

// Releases a block returned by `alloc`, `alloc_zeroed`, or `realloc`.
//
// A null pointer and a zero-size layout are no-ops, matching the runtime. For
// every real block, `layout` must be the exact layout that created the current
// pointer. The pointer is invalid after this call, and releasing the same block
// twice is undefined behavior.
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
