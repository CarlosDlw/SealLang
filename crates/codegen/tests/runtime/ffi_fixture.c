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
#include <string.h>

static int32_t fixture_stored;

/* C owns this storage; the Seal program reads and writes it through an
   `extern static mut` declaration. */
int32_t ffi_external_counter = 7;

typedef struct {
  int32_t x;
  int32_t y;
} ffi_point;

typedef struct {
  uint8_t first;
  uint8_t second;
} ffi_two;

typedef struct {
  uint8_t first;
  uint8_t second;
  uint8_t third;
} ffi_three;

typedef struct {
  uint8_t first;
  uint8_t second;
  uint8_t third;
  uint8_t fourth;
} ffi_four;

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

typedef struct {
  _Bool enabled;
} ffi_flags;

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

int32_t ffi_wchar_width(void) { return (int32_t)(sizeof(wchar_t) * 8); }

int32_t ffi_wchar_signed(void) {
  volatile wchar_t value = (wchar_t)-1;
  return value < 0;
}

int32_t ffi_bool_value(_Bool value) { return value ? 1 : 0; }

_Bool ffi_bool_identity(_Bool value) { return value; }

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

ffi_flags ffi_flags_new(_Bool enabled) {
  ffi_flags flags = {enabled};
  return flags;
}

int32_t ffi_flags_check(ffi_flags flags) { return flags.enabled ? 1 : 0; }

int32_t ffi_sizeof_flags(void) { return (int32_t)sizeof(ffi_flags); }

ffi_two ffi_two_make(uint8_t first, uint8_t second) {
  ffi_two value = {first, second};
  return value;
}

int32_t ffi_two_check(ffi_two value) {
  return value.first == 1 && value.second == 2;
}

ffi_three ffi_three_make(uint8_t first, uint8_t second, uint8_t third) {
  ffi_three value = {first, second, third};
  return value;
}

int32_t ffi_three_check(ffi_three value) {
  return value.first == 1 && value.second == 2 && value.third == 3;
}

ffi_four ffi_four_make(uint8_t first, uint8_t second, uint8_t third,
                       uint8_t fourth) {
  ffi_four value = {first, second, third, fourth};
  return value;
}

int32_t ffi_four_check(ffi_four value) {
  return value.first == 1 && value.second == 2 && value.third == 3 &&
         value.fourth == 4;
}

int32_t ffi_sizeof_two(void) { return (int32_t)sizeof(ffi_two); }

int32_t ffi_sizeof_three(void) { return (int32_t)sizeof(ffi_three); }

int32_t ffi_sizeof_four(void) { return (int32_t)sizeof(ffi_four); }

void ffi_bump_external_counter(int32_t by) { ffi_external_counter += by; }

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

typedef int32_t (*ffi_callback)(int32_t);
typedef ffi_point (*ffi_point_callback)(ffi_point);

int32_t ffi_apply(ffi_callback callback, int32_t value) {
  return callback(value);
}

void ffi_fill_buffer(char *out, size_t cap) {
  static const char text[] = "hello";
  const size_t length = sizeof(text) - 1;
  if (out == NULL || cap < length) {
    return;
  }
  memcpy(out, text, length);
}

const char *ffi_utf8_message(void) { return "ol\xc3\xa1"; }

void ffi_exit_now(int32_t code) { _Exit(code); }

int32_t ffi_register_atexit(void (*callback)(void)) { return atexit(callback); }

int32_t ffi_apply_null(ffi_callback callback) { return callback == NULL; }

int32_t ffi_signal_like(ffi_callback callback) { return callback(42); }

ffi_point ffi_apply_point(ffi_point_callback callback, ffi_point point) {
  return callback(point);
}

static int compare_i32(const void *left, const void *right) {
  int32_t a = *(const int32_t *)left;
  int32_t b = *(const int32_t *)right;
  return (a > b) - (a < b);
}

void ffi_qsort_ints(int32_t *values, int32_t count) {
  qsort(values, (size_t)count, sizeof(int32_t), compare_i32);
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
