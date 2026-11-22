module alloc;

// Private declarations for the runtime's heap ABI.
//
// These symbols are internal to the compiler and its runtime. Seal code calls
// the validated wrappers in `heap.sl`; it must never call this ABI directly.
extern "c" {
  func seal_alloc(size: usize, align: usize) -> *raw mut u8;
  func seal_alloc_zeroed(size: usize, align: usize) -> *raw mut u8;
  func seal_dealloc(pointer: *raw mut u8, size: usize, align: usize);
}
