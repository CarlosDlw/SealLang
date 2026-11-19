/*
 * Host allocator backend for the Seal heap layer.
 *
 * The backend is private to `runtime/heap/`. Keeping it behind a narrow
 * interface lets a future target or allocator replace the libc calls without
 * changing the public ABI in `seal_rt.h`.
 */

#ifndef SEAL_HEAP_SYSTEM_H
#define SEAL_HEAP_SYSTEM_H

#include <stddef.h>

/* Alignment guaranteed by the host allocator for ordinary allocations. */
#define SEAL_SYSTEM_MAX_ALIGN ((size_t)_Alignof(max_align_t))

/* Allocates `size` bytes with the host allocator's fundamental alignment. */
void *seal_system_alloc(size_t size);

/* Allocates `size` zeroed bytes with the host allocator. */
void *seal_system_alloc_zeroed(size_t size);

/* Releases a pointer returned by this backend. */
void seal_system_dealloc(void *pointer);

#endif /* SEAL_HEAP_SYSTEM_H */
