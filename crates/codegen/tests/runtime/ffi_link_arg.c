/* Supplied as an extra object through --link-arg. */

#include <stdint.h>

int32_t ffi_external_value(void) { return 42; }

int32_t ffi_link_arg_value(void) { return ffi_external_value(); }
