/*
 * Unrecoverable failure reporting for the Seal runtime.
 *
 * The output shape is stable and intentionally small. This translation unit
 * never allocates, never takes a lock, never formats through stdio, and never
 * depends on the locale, so it stays usable after the program has entered an
 * unrecoverable state.
 */

#include "seal_rt.h"

#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#else
#include <unistd.h>
#endif

#define SEAL_STDERR 2
#define SEAL_PANIC_STATUS 101

/* Writes a size-independent decimal representation into `buffer`. */
static size_t seal_decimal(uint32_t value, char *buffer) {
    char reversed[10];
    size_t count = 0;
    do {
        reversed[count++] = (char)('0' + (value % 10));
        value /= 10;
    } while (value != 0);
    for (size_t index = 0; index < count; index++) {
        buffer[index] = reversed[count - 1 - index];
    }
    return count;
}

static void seal_write_literal(int fd, const char *text, size_t length) {
    (void)seal_write(fd, (const uint8_t *)text, length);
}

static void seal_write_number(int fd, uint32_t value) {
    char buffer[10];
    size_t length = seal_decimal(value, buffer);
    (void)seal_write(fd, (const uint8_t *)buffer, length);
}

/*
 * Reports an unrecoverable runtime error and terminates the process with the
 * conventional panic status (101).
 *
 * The output shape is stable and intentionally small:
 *
 *     panic: <message> at <file>:<line>:<column>\n
 *
 * The function is `noreturn`: generated code emits an `unreachable` terminator
 * after every call, and LLVM is told the call is cold and cannot unwind.
 */
SEAL_NORETURN SEAL_EXPORT void seal_panic(const uint8_t *message,
                                          size_t message_length,
                                          const uint8_t *file,
                                          size_t file_length,
                                          int32_t line,
                                          int32_t column) {
    seal_write_literal(SEAL_STDERR, "panic: ", sizeof("panic: ") - 1);
    if (message != NULL) {
        (void)seal_write(SEAL_STDERR, message, message_length);
    }
    seal_write_literal(SEAL_STDERR, " at ", sizeof(" at ") - 1);
    if (file != NULL) {
        (void)seal_write(SEAL_STDERR, file, file_length);
    }
    seal_write_literal(SEAL_STDERR, ":", 1);
    seal_write_number(SEAL_STDERR, line > 0 ? (uint32_t)line : 0);
    seal_write_literal(SEAL_STDERR, ":", 1);
    seal_write_number(SEAL_STDERR, column > 0 ? (uint32_t)column : 0);
    seal_write_literal(SEAL_STDERR, "\n", 1);

#if defined(_WIN32)
    ExitProcess(SEAL_PANIC_STATUS);
#else
    _exit(SEAL_PANIC_STATUS);
#endif
}
