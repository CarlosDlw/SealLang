/*
 * C ABI fixture for the Seal FFI integration tests.
 *
 * Every function takes and returns scalar values with the widths documented by
 * the C ABI. ffi_link_arg_value calls a symbol supplied by an extra object on
 * the linker command line, which proves --link-arg reaches the link.
 */

#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>

static int32_t fixture_stored;

typedef struct {
  int32_t x;
  int32_t y;
} ffi_point;

typedef struct {
  ffi_point point;
  double scale;
} ffi_nested;

typedef struct {
  int32_t values[3];
} ffi_array;

typedef struct {
  int8_t small;
  int64_t wide;
  double real;
} ffi_mixed;

typedef struct {
  int32_t first;
  double second;
  ffi_point point;
  int64_t last;
} ffi_wide;

typedef struct ffi_handle {
  int32_t value;
} ffi_handle;

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

ffi_point ffi_point_new(int32_t x, int32_t y) {
  ffi_point point = {x, y};
  return point;
}

int32_t ffi_point_sum(ffi_point point) { return point.x + point.y; }

ffi_point ffi_point_make(int32_t x, int32_t y) { return ffi_point_new(x, y); }

ffi_nested ffi_nested_make(int32_t x, int32_t y, double scale) {
  ffi_nested nested = {{x, y}, scale};
  return nested;
}

double ffi_nested_sum(ffi_nested nested) {
  return (double)(nested.point.x + nested.point.y) * nested.scale;
}

ffi_array ffi_array_make(int32_t first, int32_t second, int32_t third) {
  ffi_array array = {{first, second, third}};
  return array;
}

int32_t ffi_array_sum(ffi_array array) {
  return array.values[0] + array.values[1] + array.values[2];
}

ffi_mixed ffi_mixed_make(int8_t small, int64_t wide, double real) {
  ffi_mixed mixed = {small, wide, real};
  return mixed;
}

int32_t ffi_mixed_check(ffi_mixed mixed) {
  return mixed.small == 7 && mixed.wide == 35 && mixed.real == 2.5;
}

ffi_wide ffi_wide_make(int32_t first, double second, int32_t x, int32_t y,
                       int64_t last) {
  ffi_wide wide = {first, second, {x, y}, last};
  return wide;
}

int32_t ffi_wide_check(ffi_wide wide) {
  return wide.first == 1 && wide.second == 2.5 && wide.point.x == 3 &&
         wide.point.y == 4 && wide.last == 5;
}

int32_t ffi_sizeof_point(void) { return (int32_t)sizeof(ffi_point); }

int32_t ffi_offsetof_point_y(void) { return (int32_t)offsetof(ffi_point, y); }

int32_t ffi_sizeof_nested(void) { return (int32_t)sizeof(ffi_nested); }

int32_t ffi_sizeof_array(void) { return (int32_t)sizeof(ffi_array); }

int32_t ffi_sizeof_mixed(void) { return (int32_t)sizeof(ffi_mixed); }

int32_t ffi_sizeof_wide(void) { return (int32_t)sizeof(ffi_wide); }

ffi_handle *ffi_handle_new(int32_t value) {
  ffi_handle *handle = malloc(sizeof(ffi_handle));
  if (handle == NULL) {
    return NULL;
  }
  handle->value = value;
  return handle;
}

int32_t ffi_handle_get(const ffi_handle *handle) { return handle->value; }

void ffi_handle_set(ffi_handle *handle, int32_t value) { handle->value = value; }

void ffi_handle_free(ffi_handle *handle) { free(handle); }

int32_t ffi_zero_sized_probe(void) { return 0; }

int32_t ffi_cstr_len(const char *text) {
  int32_t length = 0;
  while (text[length] != '\0') {
    length += 1;
  }
  return length;
}

int32_t ffi_sum_ints(int32_t count, ...) {
  va_list arguments;
  va_start(arguments, count);
  int32_t total = 0;
  for (int32_t index = 0; index < count; index += 1) {
    total += va_arg(arguments, int);
  }
  va_end(arguments);
  return total;
}

double ffi_sum_doubles(int32_t count, ...) {
  va_list arguments;
  va_start(arguments, count);
  double total = 0.0;
  for (int32_t index = 0; index < count; index += 1) {
    total += va_arg(arguments, double);
  }
  va_end(arguments);
  return total;
}

int64_t ffi_variadic_wide(int32_t count, ...) {
  va_list arguments;
  va_start(arguments, count);
  int64_t total = 0;
  for (int32_t index = 0; index < count; index += 1) {
    total += va_arg(arguments, int64_t);
  }
  va_end(arguments);
  return total;
}

int32_t ffi_variadic_kinds(int32_t count, ...) {
  if (count != 3) {
    return -1;
  }
  va_list arguments;
  va_start(arguments, count);
  int first = va_arg(arguments, int);
  double second = va_arg(arguments, double);
  int64_t third = va_arg(arguments, int64_t);
  va_end(arguments);
  return first == 1 && second == 2.5 && third == 3;
}

int32_t ffi_variadic_first_char(int32_t count, ...) {
  if (count < 1) {
    return -1;
  }
  va_list arguments;
  va_start(arguments, count);
  const char *text = va_arg(arguments, const char *);
  va_end(arguments);
  return (int32_t)(unsigned char)text[0];
}
