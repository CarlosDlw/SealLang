module alloc;

// The reason a requested layout is not valid.
public enum LayoutError {
  // The alignment was zero or not a power of two.
  AlignmentNotPowerOfTwo,
  // The size, or the size rounded up to the alignment, exceeds `isize::MAX`.
  SizeOverflow,
}

// The runtime could not satisfy a valid allocation request.
//
// Allocation failure is an ordinary value. The allocator never panics, aborts,
// logs, or returns a null pointer as a successful allocation.
public struct AllocError {}
