module alloc;

use core::result::Result;

// The checked layout of an allocation.
//
// A layout is the pair of a byte size and a power-of-two alignment. It is the
// value accepted by `alloc`, `alloc_zeroed`, `realloc`, and `dealloc`; it is
// also the layout stored by an owning buffer so deallocation uses exactly the
// size and alignment that created the block.
//
// `Layout::from_size_align` performs the complete validation. A layout is valid
// only when its alignment is non-zero and a power of two, and its size rounded
// up to that alignment fits in `isize::MAX`. The second rule keeps pointer
// arithmetic and the code generator's object-size model within the target's
// address space.
//
// `Layout` is `Copy`: it is a pair of `usize` values with no ownership.
public struct Layout {
  size: usize,
  align: usize,
}

impl Copy for Layout {}

impl Layout {
  // Returns the layout of a sized type.
  public func new<T>() -> Layout {
    Layout { size: size_of::<T>(), align: align_of::<T>() }
  }

  // Builds a layout from a byte size and an alignment.
  //
  // Returns `AlignmentNotPowerOfTwo` for a zero or non-power-of-two alignment,
  // and `SizeOverflow` when the size rounded up to the alignment would exceed
  // `isize::MAX`.
  public func from_size_align(size: usize, align: usize) -> Result<Layout, LayoutError> {
    if align == 0 || (align & (align - 1)) != 0 {
      return Result::Err(LayoutError::AlignmentNotPowerOfTwo);
    }

    let max = max_isize();
    if size > max - (align - 1) {
      return Result::Err(LayoutError::SizeOverflow);
    }

    Result::Ok(Layout { size: size, align: align })
  }

  // Returns the layout of `count` consecutive `T` values.
  //
  // The multiplication is checked before the layout is validated, so a
  // zero-sized element can be repeated any number of times without overflow.
  public func array<T>(count: usize) -> Result<Layout, LayoutError> {
    let element_size = size_of::<T>();
    let element_align = align_of::<T>();
    let max = max_isize();
    if element_size != 0 && count > max / element_size {
      return Result::Err(LayoutError::SizeOverflow);
    }

    Layout::from_size_align(element_size * count, element_align)
  }

  // Returns the number of bytes in the layout.
  public func size(self: Layout) -> usize {
    self.size
  }

  // Returns the alignment of the layout.
  public func align(self: Layout) -> usize {
    self.align
  }
}

// Returns the largest value of `isize`, calculated without an associated
// constant so the result follows the target pointer width.
func max_isize() -> usize {
  (0usize.wrapping_sub(1)) >> 1
}
