/*
 * Aligned allocation ABI for the Seal heap layer.
 *
 * The ABI validates alignment and layout without trusting the caller, returns
 * NULL on failure, and never panics or aborts. Alignments up to
 * `max_align_t` use the host allocator directly. Larger alignments
 * over-allocate, align the user pointer, and store the base pointer in the word
 * immediately before it, which keeps reallocation and deallocation portable
 * without `posix_memalign`, `aligned_alloc`, or `_aligned_malloc`.
 */

#include "seal_rt.h"
#include "system.h"

#include <stdint.h>
#include <string.h>

/* One pointer-sized word immediately before an over-aligned user pointer. */
#define SEAL_HEAP_HEADER ((size_t)sizeof(void *))

static int seal_valid_align(size_t align) {
    return align != 0 && (align & (align - 1)) == 0;
}

static void *seal_dangling(size_t align) {
    return (void *)(uintptr_t)align;
}

static int seal_needs_header(size_t align) {
    return align > SEAL_SYSTEM_MAX_ALIGN;
}

/*
 * Computes the raw allocation size for an over-aligned block. Returns zero when
 * the size arithmetic would overflow `size_t`; the caller turns that into a
 * NULL result.
 */
static int seal_raw_size(size_t size, size_t align, size_t *raw_size) {
    size_t slack;
    if (size > SIZE_MAX - (align - 1)) {
        return 0;
    }
    slack = size + (align - 1);
    if (slack > SIZE_MAX - SEAL_HEAP_HEADER) {
        return 0;
    }
    *raw_size = slack + SEAL_HEAP_HEADER;
    return 1;
}

/* Computes the aligned user pointer without touching its header word. */
static void *seal_aligned_pointer(void *base, size_t align) {
    uintptr_t start = (uintptr_t)base + SEAL_HEAP_HEADER;
    uintptr_t mask = (uintptr_t)align - 1;
    uintptr_t aligned = (start + mask) & ~mask;
    return (void *)aligned;
}

/* Aligns the user pointer and stores the base pointer in its header word. */
static void *seal_align(void *base, size_t align) {
    void *pointer = seal_aligned_pointer(base, align);
    ((void **)pointer)[-1] = base;
    return pointer;
}

SEAL_EXPORT void *seal_alloc(size_t size, size_t align) {
    if (!seal_valid_align(align)) {
        return NULL;
    }
    if (size == 0) {
        return seal_dangling(align);
    }
    if (!seal_needs_header(align)) {
        return seal_system_alloc(size);
    }

    size_t raw_size;
    if (!seal_raw_size(size, align, &raw_size)) {
        return NULL;
    }
    void *base = seal_system_alloc(raw_size);
    if (base == NULL) {
        return NULL;
    }
    return seal_align(base, align);
}

SEAL_EXPORT void *seal_alloc_zeroed(size_t size, size_t align) {
    if (!seal_valid_align(align)) {
        return NULL;
    }
    if (size == 0) {
        return seal_dangling(align);
    }
    if (!seal_needs_header(align)) {
        return seal_system_alloc_zeroed(size);
    }

    size_t raw_size;
    if (!seal_raw_size(size, align, &raw_size)) {
        return NULL;
    }
    void *base = seal_system_alloc(raw_size);
    if (base == NULL) {
        return NULL;
    }
    void *user = seal_align(base, align);
    memset(user, 0, size);
    return user;
}

SEAL_EXPORT void *seal_realloc(void *pointer, size_t old_size, size_t align,
                               size_t new_size) {
    if (!seal_valid_align(align)) {
        return NULL;
    }
    if (pointer == NULL) {
        return seal_alloc(new_size, align);
    }
    if (new_size == 0) {
        seal_dealloc(pointer, old_size, align);
        return seal_dangling(align);
    }
    if (!seal_needs_header(align)) {
        return seal_system_realloc(pointer, new_size);
    }

    size_t raw_size;
    if (!seal_raw_size(new_size, align, &raw_size)) {
        return NULL;
    }
    void *base = ((void **)pointer)[-1];
    size_t old_offset = (size_t)((uintptr_t)pointer - (uintptr_t)base);
    void *new_base = seal_system_realloc(base, raw_size);
    if (new_base == NULL) {
        return NULL;
    }

    void *new_pointer = seal_aligned_pointer(new_base, align);
    size_t copied = old_size < new_size ? old_size : new_size;
    memmove(new_pointer, (unsigned char *)new_base + old_offset, copied);
    ((void **)new_pointer)[-1] = new_base;
    return new_pointer;
}

SEAL_EXPORT void seal_dealloc(void *pointer, size_t size, size_t align) {
    if (pointer == NULL || size == 0) {
        return;
    }
    if (seal_needs_header(align)) {
        seal_system_dealloc(((void **)pointer)[-1]);
    } else {
        seal_system_dealloc(pointer);
    }
}
