/*
 * Process termination for the Seal runtime.
 *
 * `seal_exit` is the runtime counterpart of the `exit(code)` built-in. It uses
 * the platform's direct process-exit primitive: no stdio is flushed, no atexit
 * handler runs, and no destructor is invoked, because Seal has no unwinding and
 * records cleanup explicitly. The caller is responsible for running the drops
 * that belong before the exit.
 */

#include "seal_rt.h"

#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#else
#include <unistd.h>
#endif

SEAL_NORETURN SEAL_EXPORT void seal_exit(int32_t code) {
#if defined(_WIN32)
    ExitProcess((UINT)(uint32_t)code);
#else
    _exit((int)code);
#endif
}
