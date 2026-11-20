/* A library that depends on a symbol from the next library on the link line. */

#include <stdint.h>

int32_t ffi_dependency_value(void);

int32_t ffi_dependent_value(void) { return ffi_dependency_value(); }
