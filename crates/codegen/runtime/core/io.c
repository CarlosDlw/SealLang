/*
 * Process output primitives for the Seal runtime.
 *
 * Output uses the operating system directly so a panic cannot be lost in a
 * stdio buffer. This translation unit never allocates, never takes a lock, and
 * never depends on the locale.
 */

#include "seal_rt.h"

#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#else
#include <errno.h>
#include <unistd.h>
#endif

#define SEAL_STDOUT 1
#define SEAL_STDERR 2

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
