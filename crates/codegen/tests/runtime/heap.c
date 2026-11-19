/*
 * Contract tests for the Seal heap ABI.
 *
 * This program is linked against the embedded `seal_rt` archive and never
 * includes the runtime sources directly. It checks allocation, alignment,
 * zeroing, zero-size behavior, invalid parameters, layout overflow, and
 * deallocation; it does not test reallocation, which is a later phase.
 */

#include "seal_rt.h"

#include <stdint.h>
#include <stdio.h>
#include <string.h>

static int failures = 0;

static void check(int condition, const char *expression, const char *file, int line) {
    if (!condition) {
        fprintf(stderr, "%s:%d: check failed: %s\n", file, line, expression);
        failures++;
    }
}

#define CHECK(expression) check((expression), #expression, __FILE__, __LINE__)

static int is_aligned(const void *pointer, size_t align) {
    return ((uintptr_t)pointer & (align - 1)) == 0;
}

static void fill(unsigned char *data, size_t length, unsigned char seed) {
    for (size_t index = 0; index < length; index++) {
        data[index] = (unsigned char)(seed + (unsigned char)index);
    }
}

static int matches(const unsigned char *data, size_t length, unsigned char seed) {
    for (size_t index = 0; index < length; index++) {
        if (data[index] != (unsigned char)(seed + (unsigned char)index)) {
            return 0;
        }
    }
    return 1;
}

static void check_block(size_t size, size_t align) {
    unsigned char *pointer = (unsigned char *)seal_alloc(size, align);
    CHECK(pointer != NULL);
    if (pointer == NULL) {
        return;
    }
    CHECK(is_aligned(pointer, align));
    fill(pointer, size, (unsigned char)(size + align));
    CHECK(matches(pointer, size, (unsigned char)(size + align)));
    seal_dealloc(pointer, size, align);
}

int main(void) {
    unsigned char *byte = (unsigned char *)seal_alloc(1, 1);
    CHECK(byte != NULL);
    if (byte != NULL) {
        byte[0] = 0xA5;
        CHECK(byte[0] == 0xA5);
        seal_dealloc(byte, 1, 1);
    }

    check_block(1, 2);
    check_block(64, 8);
    check_block(64, 16);
    check_block(64, 32);
    check_block(64, 64);
    check_block(64, 128);
    check_block(64, 4096);
    check_block(1, 4096);
    check_block(1, 8192);

    CHECK(seal_alloc(64, 0) == NULL);
    CHECK(seal_alloc(64, 3) == NULL);
    CHECK(seal_alloc(64, 6) == NULL);
    CHECK(seal_alloc(0, 0) == NULL);

    void *empty = seal_alloc(0, 8);
    CHECK(empty != NULL);
    CHECK((uintptr_t)empty == 8);
    seal_dealloc(empty, 0, 8);

    void *empty_aligned = seal_alloc(0, 4096);
    CHECK(empty_aligned != NULL);
    CHECK(is_aligned(empty_aligned, 4096));
    seal_dealloc(empty_aligned, 0, 4096);

    void *empty_zeroed = seal_alloc_zeroed(0, 8);
    CHECK(empty_zeroed != NULL);
    CHECK((uintptr_t)empty_zeroed == 8);
    seal_dealloc(empty_zeroed, 0, 8);

    void *zeroed = seal_alloc_zeroed(256, 8);
    CHECK(zeroed != NULL);
    if (zeroed != NULL) {
        const unsigned char *bytes = (const unsigned char *)zeroed;
        int all_zero = 1;
        for (size_t index = 0; index < 256; index++) {
            if (bytes[index] != 0) {
                all_zero = 0;
            }
        }
        CHECK(all_zero);
        seal_dealloc(zeroed, 256, 8);
    }

    void *zeroed_aligned = seal_alloc_zeroed(256, 64);
    CHECK(zeroed_aligned != NULL);
    if (zeroed_aligned != NULL) {
        const unsigned char *bytes = (const unsigned char *)zeroed_aligned;
        int all_zero = 1;
        CHECK(is_aligned(zeroed_aligned, 64));
        for (size_t index = 0; index < 256; index++) {
            if (bytes[index] != 0) {
                all_zero = 0;
            }
        }
        CHECK(all_zero);
        seal_dealloc(zeroed_aligned, 256, 64);
    }

    CHECK(seal_alloc(SIZE_MAX, 4096) == NULL);
    CHECK(seal_alloc(SIZE_MAX - 64, 4096) == NULL);
    CHECK(seal_alloc_zeroed(SIZE_MAX, 4096) == NULL);

    seal_dealloc(NULL, 0, 8);
    seal_dealloc(NULL, 16, 8);

    for (int iteration = 0; iteration < 10000; iteration++) {
        size_t size = (size_t)(iteration % 257) + 1;
        size_t align = (size_t)1 << (iteration % 7);
        check_block(size, align);
    }

    if (failures != 0) {
        fprintf(stderr, "%d heap check(s) failed\n", failures);
        return 1;
    }
    printf("heap ABI: ok\n");
    return 0;
}
