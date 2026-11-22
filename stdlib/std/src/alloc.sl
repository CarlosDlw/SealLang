module alloc;

// The hosted allocation substrate.
//
// `std::alloc` is the language-level layer above the runtime's internal heap
// ABI. It provides validated layouts, allocation failure as a value, and
// unsafe raw allocation and deallocation for containers and other callers that
// manage their own storage.
//
// Every block returned by `alloc` or `alloc_zeroed` is owned by the caller and
// must be released exactly once with `dealloc` and the same layout. Seal blocks
// must never be passed to C `free`.
//
// This module belongs to the hosted `std` package. `core` remains free of
// heap, operating-system, and runtime dependencies; a freestanding target
// cannot select `std`, and `--no-stdlib` has no allocation API.
