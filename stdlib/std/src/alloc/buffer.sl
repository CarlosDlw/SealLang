module alloc;

use core::result::Result;

// An owning, untyped block of bytes.
//
// A `RawBuffer` stores the exact `Layout` that created its current block and
// releases it exactly once when the value is dropped. It never duplicates
// ownership and is deliberately not `Copy`.
//
// The buffer owns bytes, not initialized typed values. It therefore exposes no
// safe view into the block. `as_ptr` returns a raw pointer, and every read or
// write through that pointer remains `unsafe`.
//
// An empty buffer has a null pointer, zero capacity, and a zero-size layout;
// dropping it or resizing it to zero releases nothing. A failed resize leaves
// the old block and layout untouched.
public struct RawBuffer {
  pointer: *raw mut u8,
  layout: Layout,
}

impl RawBuffer {
  // Returns an empty buffer with null `as_ptr` and zero capacity.
  //
  // The empty buffer uses alignment 1. Use `new` when the alignment of a future
  // allocation is already known.
  public func empty() -> RawBuffer {
    RawBuffer { pointer: null_pointer(), layout: zero_layout(1) }
  }

  // Allocates one uninitialized block for `layout`.
  //
  // A zero-size layout produces an empty buffer: null `as_ptr`, zero capacity,
  // and the given zero-size layout. A runtime allocation failure is returned as
  // `AllocError` without changing any state.
  public func new(layout: Layout) -> Result<RawBuffer, AllocError> {
    if layout.size() == 0 {
      return Result::Ok(RawBuffer { pointer: null_pointer(), layout: layout });
    }

    match unsafe { alloc(layout) } {
      Result::Ok(pointer) => Result::Ok(RawBuffer { pointer: pointer, layout: layout }),
      Result::Err(error) => Result::Err(error),
    }
  }

  // Returns the exact layout of the current block.
  //
  // For an empty buffer the size is zero and the alignment is retained from the
  // layout that was last owned, or 1 for `empty`.
  public func layout(self: *RawBuffer) -> Layout {
    (*self).layout
  }

  // Returns the number of bytes in the current block.
  public func capacity(self: *RawBuffer) -> usize {
    (*self).layout.size()
  }

  // Returns the raw pointer to the first byte of the block.
  //
  // The pointer is null when the buffer is empty. Otherwise it is aligned to
  // `layout().align()` and valid for `capacity()` bytes. Treat it as invalid
  // after `resize`, `Drop`, or a move of the owning value.
  public func as_ptr(self: *RawBuffer) -> *raw mut u8 {
    (*self).pointer
  }

  // Resizes the block to `new_size` bytes while preserving its alignment.
  //
  // Growing preserves the first `min(old_size, new_size)` bytes and leaves the
  // new tail uninitialized. Shrinking preserves the first `new_size` bytes.
  // Resizing to zero releases any real block and returns to the empty state
  // while retaining the alignment for a later allocation.
  //
  // A size that cannot form a layout, or a runtime allocation failure, returns
  // `AllocError` and leaves the old pointer, layout, and bytes intact.
  public func resize(self: *mut RawBuffer, new_size: usize) -> Result<(), AllocError> {
    let old_layout = (*self).layout;
    let old_size = old_layout.size();

    if new_size == old_size {
      return Result::Ok(());
    }

    if new_size == 0 {
      unsafe { dealloc((*self).pointer, old_layout); }
      (*self).pointer = null_pointer();
      (*self).layout = zero_layout(old_layout.align());
      return Result::Ok(());
    }

    let new_layout = match Layout::from_size_align(new_size, old_layout.align()) {
      Result::Ok(layout) => layout,
      Result::Err(_) => {
        return Result::Err(AllocError {});
      }
    };

    if old_size == 0 {
      match unsafe { alloc(new_layout) } {
        Result::Ok(pointer) => {
          (*self).pointer = pointer;
          (*self).layout = new_layout;
          Result::Ok(())
        }
        Result::Err(error) => Result::Err(error),
      }
    } else {
      match unsafe { realloc((*self).pointer, old_layout, new_size) } {
        Result::Ok(pointer) => {
          (*self).pointer = pointer;
          (*self).layout = new_layout;
          Result::Ok(())
        }
        Result::Err(error) => Result::Err(error),
      }
    }
  }
}

impl Drop for RawBuffer {
  func drop(self: *mut RawBuffer) {
    unsafe { dealloc((*self).pointer, (*self).layout); }
  }
}

func null_pointer() -> *raw mut u8 {
  0usize as *raw mut u8
}

func zero_layout(align: usize) -> Layout {
  Layout { size: 0, align: align }
}
