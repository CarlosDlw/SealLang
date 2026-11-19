/*
 * Host allocator backend for the Seal heap layer.
 *
 * This translation unit is the only place that calls the C allocator. It does
 * not validate language-level layout rules and does not panic: the heap ABI in
 * `alloc.c` owns the contract.
 */

#include "system.h"

#include <stdlib.h>

void *seal_system_alloc(size_t size) {
    return malloc(size);
}

void *seal_system_alloc_zeroed(size_t size) {
    return calloc(1, size);
}

void seal_system_dealloc(void *pointer) {
    free(pointer);
}
