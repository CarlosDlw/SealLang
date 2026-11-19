/*
 * Seal runtime ABI: process primitives shared by generated programs and the
 * native runtime implementation.
 *
 * This header is internal to the compiler and its runtime. It is not a
 * language-level API and must not be exposed to Seal source.
 *
 * The ABI uses separate data/length pairs for strings: the compiler represents
 * `*str` as a fat pointer, but passing two scalars keeps the boundary free of
 * struct-by-value layout questions between LLVM and the C compiler.
 */

#ifndef SEAL_RT_H
#define SEAL_RT_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#if defined(_WIN32)
#define SEAL_NORETURN __declspec(noreturn)
#define SEAL_EXPORT
#else
#define SEAL_NORETURN _Noreturn
#define SEAL_EXPORT __attribute__((visibility("default")))
#endif

/*
 * Writes exactly `length` bytes to `fd`, retrying interrupted calls and
 * continuing after partial writes. Returns 0 on success and -1 when the output
 * cannot be completed.
 */
SEAL_EXPORT int seal_write(int fd, const uint8_t *data, size_t length);

/* Writes to standard output. */
SEAL_EXPORT int seal_print(const uint8_t *data, size_t length);

/* Writes to standard error. */
SEAL_EXPORT int seal_eprint(const uint8_t *data, size_t length);

/*
 * Reports an unrecoverable runtime error and terminates the process with the
 * conventional panic status (101). The function never returns, never unwinds,
 * and never allocates.
 */
SEAL_NORETURN SEAL_EXPORT void seal_panic(const uint8_t *message,
                                          size_t message_length,
                                          const uint8_t *file,
                                          size_t file_length,
                                          int32_t line,
                                          int32_t column);

/*
 * Allocates `size` bytes aligned to `align`, which must be a power of two.
 * Returns NULL for invalid alignment, layout overflow, or allocation failure.
 * A zero size returns a non-null dangling pointer equal to `align` without
 * allocating; it must never be dereferenced.
 */
SEAL_EXPORT void *seal_alloc(size_t size, size_t align);

/* Same contract as seal_alloc, with every allocated byte zeroed. */
SEAL_EXPORT void *seal_alloc_zeroed(size_t size, size_t align);

/*
 * Releases a block returned by this ABI. `pointer == NULL` and `size == 0` are
 * no-ops. `align` must be the alignment passed to seal_alloc or
 * seal_alloc_zeroed for this block.
 */
SEAL_EXPORT void seal_dealloc(void *pointer, size_t size, size_t align);

#ifdef __cplusplus
}
#endif

#endif /* SEAL_RT_H */
