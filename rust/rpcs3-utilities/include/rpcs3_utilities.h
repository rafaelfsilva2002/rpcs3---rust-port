/* rpcs3-utilities: stable C ABI exposed to the C++ emulator.
 *
 * This header is hand-written for Phase 0 (small surface, one function).
 * Once the crate grows beyond a handful of symbols, switch to cbindgen
 * via `cbindgen --config cbindgen.toml --output include/rpcs3_utilities.h`.
 */

#ifndef RPCS3_UTILITIES_H
#define RPCS3_UTILITIES_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/**
 * Returns the file extension of `file_path`, without the leading '.'.
 *
 * Semantics mirror Utilities/StrFmt.cpp:958 `get_file_extension`:
 *  - Finds the last '.' in the path.
 *  - If none, or if '.' is the last character, returns empty.
 *  - Otherwise returns the substring after the last '.'.
 *
 * The caller passes a buffer `out` of capacity `out_cap`. The function
 * writes at most `out_cap` bytes (no NUL is appended by the Rust side).
 * Returns the number of bytes that would be written if the buffer were
 * large enough — caller compares with `out_cap` to detect truncation.
 *
 * `file_path` is a pointer to `file_path_len` bytes (not required to
 * be NUL-terminated). Must not be NULL when `file_path_len > 0`.
 */
size_t rpcs3_get_file_extension(
    const char* file_path,
    size_t file_path_len,
    char* out,
    size_t out_cap);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* RPCS3_UTILITIES_H */
