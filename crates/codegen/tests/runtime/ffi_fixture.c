/*
 * C ABI fixture for the Seal FFI integration tests.
 *
 * Every function takes and returns scalar values with the widths documented by
 * the C ABI. ffi_link_arg_value calls a symbol supplied by an extra object on
 * the linker command line, which proves --link-arg reaches the link.
 */

#include <stddef.h>
#include <stdint.h>

static int32_t fixture_stored;

int32_t ffi_scalar_i32(int32_t value) { return value + 1; }

uint64_t ffi_scalar_u64(uint64_t value) { return value * 2u; }

double ffi_scalar_f64(double value) { return value + 0.5; }

void *ffi_ptr_roundtrip(void *pointer) { return pointer; }

void ffi_set_int(int32_t value) { fixture_stored = value; }

int32_t ffi_get_int(void) { return fixture_stored; }

int32_t ffi_null_is_null(void *pointer) { return pointer == NULL; }

int32_t ffi_char_signed(void) {
  volatile char value = (char)-1;
  return value < 0;
}

int32_t ffi_long_width(void) { return (int32_t)(sizeof(long) * 8); }
