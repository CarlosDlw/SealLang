module alloc;

// The hosted allocation substrate.
//
// `std::alloc` is the language-level layer above the runtime's internal heap
// ABI. It provides validated layouts, allocation failure as a value, unsafe
// raw allocation, reallocation, and deallocation for callers that manage their
// own storage, plus `RawBuffer` as the safe owner of an untyped block.
//
// Every block returned by `alloc` or `alloc_zeroed` is owned by the caller and
// must be released exactly once with `dealloc` and the same layout. A block
// passed to `realloc` keeps its old pointer valid when the resize fails. Seal
// blocks must never be passed to C `free`.
//
// This module belongs to the hosted `std` package. `core` remains free of
// heap, operating-system, and runtime dependencies; a freestanding target
// cannot select `std`, and `--no-stdlib` has no allocation API.
