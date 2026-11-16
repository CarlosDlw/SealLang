/*
 * Seal runtime: the small set of process-level primitives that generated
 * programs need independently of the host C library's stdio.
 *
 * The runtime never allocates, never takes a lock, never formats through
 * stdio, and never depends on the locale. That makes every entry point safe to
 * call after the program has entered an unrecoverable state, including from a
 * signal handler on Unix. Output uses the operating system directly so a panic
 * cannot be lost in a stdio buffer.
 *
 * The ABI uses separate data/length pairs for strings: the compiler represents
 * `*str` as a fat pointer, but passing two scalars keeps the boundary free of
 * struct-by-value layout questions between LLVM and the C compiler.
 */

#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#else
#include <errno.h>
#include <unistd.h>
#endif

#include <stddef.h>
#include <stdint.h>

#if defined(_WIN32)
#define SEAL_NORETURN __declspec(noreturn)
#define SEAL_EXPORT
#else
#define SEAL_NORETURN _Noreturn
#define SEAL_EXPORT __attribute__((visibility("default")))
#endif

#define SEAL_STDOUT 1
#define SEAL_STDERR 2

#define SEAL_PANIC_STATUS 101

/* Linux transfers at most this many bytes in one write; chunking keeps the
 * request portable to every POSIX platform and avoids EINVAL for huge buffers. */
#define SEAL_MAX_WRITE ((size_t)0x7ffff000)

/*
 * Writes exactly `length` bytes to `fd`, retrying interrupted calls and
 * continuing after partial writes. Returns 0 on success and -1 when the output
 * cannot be completed.
 *
 * A partial write is normal for pipes and sockets, and POSIX permits `write` to
 * return EINTR only when no bytes were transferred. Treating either as success
 * would silently truncate compiler diagnostics.
 */
SEAL_EXPORT int seal_write(int fd, const uint8_t *data, size_t length) {
    if (length == 0) {
        return 0;
    }
    if (data == NULL) {
        return -1;
    }

#if defined(_WIN32)
    HANDLE handle;
    switch (fd) {
    case SEAL_STDOUT:
        handle = GetStdHandle(STD_OUTPUT_HANDLE);
        break;
    case SEAL_STDERR:
        handle = GetStdHandle(STD_ERROR_HANDLE);
        break;
    default:
        return -1;
    }
    if (handle == NULL || handle == INVALID_HANDLE_VALUE) {
        return -1;
    }

    while (length > 0) {
        DWORD chunk = length > (size_t)UINT32_MAX ? UINT32_MAX : (DWORD)length;
        DWORD written = 0;
        if (!WriteFile(handle, data, chunk, &written, NULL) || written == 0) {
            return -1;
        }
        data += written;
        length -= written;
    }
    return 0;
#else
    while (length > 0) {
        size_t chunk = length > SEAL_MAX_WRITE ? SEAL_MAX_WRITE : length;
        ssize_t written = write(fd, data, chunk);
        if (written < 0) {
            if (errno == EINTR) {
                continue;
            }
            return -1;
        }
        if (written == 0) {
            return -1;
        }
        data += (size_t)written;
        length -= (size_t)written;
    }
    return 0;
#endif
}

/* Writes to standard output. */
SEAL_EXPORT int seal_print(const uint8_t *data, size_t length) {
    return seal_write(SEAL_STDOUT, data, length);
}

/* Writes to standard error. */
SEAL_EXPORT int seal_eprint(const uint8_t *data, size_t length) {
    return seal_write(SEAL_STDERR, data, length);
}

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
