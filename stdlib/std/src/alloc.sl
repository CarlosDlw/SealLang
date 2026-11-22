module alloc;

// The hosted allocation substrate.
//
// `std::alloc` is the language-level layer above the runtime's internal
// `seal_alloc`/`seal_realloc`/`seal_dealloc` ABI. It provides validated
// layouts, allocation failure as a value, and an owning untyped buffer for the
// containers that will be built on top of it.
//
// This module belongs to the hosted `std` package. `core` remains free of
// heap, operating-system, and runtime dependencies; a freestanding target
// cannot select `std`, and `--no-stdlib` has no allocation API.
