//! `rpcs3-hle-sys-libc` — PS3 libc-style HLE primitives.
//!
//! Ports `rpcs3/Emu/Cell/Modules/sys_libc.cpp` and
//! `rpcs3/Emu/Cell/Modules/sys_libc_.cpp` (prefixed `_sys_*` and
//! `__sys_*` variants registered under `sysPrxForUser`).
//!
//! Unlike most HLE modules, the firmware implementation of these
//! functions is **not** a stub — they are straight byte-for-byte mirrors
//! of the C standard library's memory/string/format primitives.  The
//! Rust port implements the same semantics against native slices so
//! higher layers can test the PS3-visible behaviour without touching
//! guest memory.
//!
//! ## Entry points covered
//!
//! | PS3 name                    | Rust wrapper                 | Source file    |
//! |-----------------------------|------------------------------|----------------|
//! | `sys_libc_memcpy`           | [`memcpy`]                   | sys_libc.cpp   |
//! | `sys_libc_memset`           | [`memset`]                   | sys_libc.cpp   |
//! | `sys_libc_memmove`          | [`memmove`]                  | sys_libc.cpp   |
//! | `sys_libc_memcmp`           | [`memcmp_u32`]               | sys_libc.cpp   |
//! | `__sys_look_ctype_table`    | [`look_ctype_table`]         | sys_libc_.cpp  |
//! | `_sys_tolower` / `_sys_toupper` | [`tolower`] / [`toupper`]| sys_libc_.cpp  |
//! | `_sys_memchr`               | [`memchr`]                   | sys_libc_.cpp  |
//! | `_sys_strlen`               | [`strlen`]                   | sys_libc_.cpp  |
//! | `_sys_strcmp` / `_sys_strncmp` | [`strcmp`] / [`strncmp`]  | sys_libc_.cpp  |
//! | `_sys_strcpy` / `_sys_strncpy` | [`strcpy`] / [`strncpy`]  | sys_libc_.cpp  |
//! | `_sys_strcat` / `_sys_strncat` | [`strcat`] / [`strncat`]  | sys_libc_.cpp  |
//! | `_sys_strchr` / `_sys_strrchr` | [`strchr`] / [`strrchr`]  | sys_libc_.cpp  |
//! | `_sys_strncasecmp`          | [`strncasecmp`]              | sys_libc_.cpp  |

extern crate alloc;

use rpcs3_emu_types::CellError;

// =====================================================================
// sys_libc.cpp — 4 primitives
// =====================================================================

/// Port of `sys_libc_memcpy` — returns `dst` unchanged after copying.
///
/// # Panics
/// If `dst.len() < size` or `src.len() < size`.
pub fn memcpy(dst: &mut [u8], src: &[u8], size: usize) {
    dst[..size].copy_from_slice(&src[..size]);
}

/// Port of `sys_libc_memset` — takes a 32-bit value but only the low
/// byte is written (matches `::memset(…, value, …)`).
pub fn memset(dst: &mut [u8], value: i32, size: usize) {
    dst[..size].fill(value as u8);
}

/// Port of `sys_libc_memmove` — overlap-safe copy.
pub fn memmove(buf: &mut [u8], src_off: usize, dst_off: usize, size: usize) {
    buf.copy_within(src_off..src_off + size, dst_off);
}

/// Port of `sys_libc_memcmp` — the PS3 entry returns `u32`, which is
/// `int` in C.  Positive for `buf1 > buf2`, negative (as u32 = large
/// positive) for `buf1 < buf2`, zero for equal.
#[must_use]
pub fn memcmp_u32(buf1: &[u8], buf2: &[u8], size: usize) -> u32 {
    memcmp(buf1, buf2, size) as u32
}

/// Common signed compare used by both `sys_libc_memcmp` and
/// `_sys_memcmp` (the C stdlib semantics).
#[must_use]
pub fn memcmp(buf1: &[u8], buf2: &[u8], size: usize) -> i32 {
    for i in 0..size {
        match buf1[i].cmp(&buf2[i]) {
            core::cmp::Ordering::Less    => return -1,
            core::cmp::Ordering::Greater => return 1,
            core::cmp::Ordering::Equal   => (),
        }
    }
    0
}

// =====================================================================
// sys_libc_.cpp — ctype table (byte-exact 129-entry table)
// =====================================================================

/// Exact port of `s_ctype_table` from sys_libc_.cpp:71-87.  Index 0 is
/// the sentinel for `ch = -1`; indices 1..=128 correspond to characters
/// 0..=127.  Bit layout: `0x01` = lower, `0x02` = upper, `0x04` = digit,
/// `0x08` = control (tab/backspace/…), `0x10` = punctuation, `0x20` =
/// space/control, `0x40` = hex-letter.
pub const CTYPE_TABLE: [i16; 129] = [
    0,
    0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x408,
    8, 8, 8, 8,
    0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20,
    0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20,
    0x18,
    0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
    0x41, 0x41, 0x41, 0x41, 0x41, 0x41,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    0x10, 0x10, 0x10, 0x10, 0x10, 0x10,
    0x42, 0x42, 0x42, 0x42, 0x42, 0x42,
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2,
    0x10, 0x10, 0x10, 0x10, 0x20,
];

/// Port of `__sys_look_ctype_table`.
///
/// # Panics
/// If `ch` is outside `-1..=127` (matches the PS3 `ensure()` abort).
#[must_use]
pub fn look_ctype_table(ch: i32) -> i16 {
    assert!((-1..=127).contains(&ch), "look_ctype_table: ch out of range: {ch}");
    CTYPE_TABLE[(ch + 1) as usize]
}

/// Port of `_sys_tolower`.  Bit 0 (`0x01`) marks upper-case letters (A..=Z
/// have entry `0x01`? — actually `A..=F` are 0x41, `G..=Z` are 0x01 in
/// the table; the low bit is set for both).
///
/// # Panics
/// If `ch` is outside `-1..=127`.
#[must_use]
pub fn tolower(ch: i32) -> i32 {
    assert!((-1..=127).contains(&ch));
    if CTYPE_TABLE[(ch + 1) as usize] & 1 != 0 { ch + 0x20 } else { ch }
}

/// Port of `_sys_toupper`.
///
/// # Panics
/// If `ch` is outside `-1..=127`.
#[must_use]
pub fn toupper(ch: i32) -> i32 {
    assert!((-1..=127).contains(&ch));
    if CTYPE_TABLE[(ch + 1) as usize] & 2 != 0 { ch - 0x20 } else { ch }
}

// =====================================================================
// sys_libc_.cpp — string / memory primitives
// =====================================================================

/// Port of `_sys_memchr`.  Returns the offset of the first `ch` byte
/// or `None` if not found.  Matches the PS3 null-pointer short-circuit
/// (`if (!buf) return vm::null`).
#[must_use]
pub fn memchr(buf: Option<&[u8]>, ch: u8, size: i32) -> Option<usize> {
    let buf = buf?;
    let size = size.max(0) as usize;
    buf.iter().take(size).position(|&b| b == ch)
}

/// Port of `_sys_strlen`.  Returns `0` if `str` is `None` (mirror of the
/// PS3 null-pointer short-circuit).
#[must_use]
pub fn strlen(s: Option<&[u8]>) -> u32 {
    let Some(s) = s else { return 0 };
    s.iter().take_while(|&&b| b != 0).count() as u32
}

/// Port of `_sys_strcmp`.  Returns -1, 0, or 1.
#[must_use]
pub fn strcmp(a: &[u8], b: &[u8]) -> i32 {
    let mut i = 0;
    loop {
        let ca = a.get(i).copied().unwrap_or(0);
        let cb = b.get(i).copied().unwrap_or(0);
        if ca < cb { return -1; }
        if ca > cb { return 1; }
        if ca == 0 { return 0; }
        i += 1;
    }
}

/// Port of `_sys_strncmp`.
#[must_use]
pub fn strncmp(a: &[u8], b: &[u8], max: u32) -> i32 {
    for i in 0..max as usize {
        let ca = a.get(i).copied().unwrap_or(0);
        let cb = b.get(i).copied().unwrap_or(0);
        if ca < cb { return -1; }
        if ca > cb { return 1; }
        if ca == 0 { break; }
    }
    0
}

/// Port of `_sys_strcat`.  Appends a NUL-terminated `src` to the end of
/// the NUL-terminated region in `dst`.  The caller must ensure `dst`
/// has room for `strlen(dst) + strlen(src) + 1`.
pub fn strcat(dst: &mut [u8], src: &[u8]) {
    let end = strlen(Some(dst)) as usize;
    let mut i = 0;
    while let Some(&b) = src.get(i) {
        dst[end + i] = b;
        if b == 0 { return; }
        i += 1;
    }
    // Reached end of src without NUL; terminate anyway.
    dst[end + i] = 0;
}

/// Port of `_sys_strchr`.  Returns `Some(offset)` or `None`.
#[must_use]
pub fn strchr(s: &[u8], ch: u8) -> Option<usize> {
    for (i, &b) in s.iter().enumerate() {
        if b == ch { return Some(i); }
        if b == 0  { return None; }
    }
    None
}

/// Port of `_sys_strncat`.  Appends up to `max` bytes then NUL-terminates.
pub fn strncat(dst: &mut [u8], src: &[u8], max: u32) {
    let end = strlen(Some(dst)) as usize;
    let max = max as usize;
    for i in 0..max {
        let Some(&b) = src.get(i) else { break };
        dst[end + i] = b;
        if b == 0 { return; }
    }
    dst[end + max] = 0;
}

/// Port of `_sys_strcpy`.  Copies `src` (NUL-terminated) into `dst`.
pub fn strcpy(dst: &mut [u8], src: &[u8]) {
    let mut i = 0;
    loop {
        let b = src.get(i).copied().unwrap_or(0);
        dst[i] = b;
        if b == 0 { return; }
        i += 1;
    }
}

/// Port of `_sys_strncpy`.  Pads remaining bytes with NUL after end of
/// source.  Returns `None` if either pointer is null.
pub fn strncpy(dst: Option<&mut [u8]>, src: Option<&[u8]>, len: i32) -> bool {
    let (Some(dst), Some(src)) = (dst, src) else { return false };
    let len = len.max(0) as usize;
    for i in 0..len {
        let b = src.get(i).copied().unwrap_or(0);
        dst[i] = b;
        if b == 0 {
            for d in dst.iter_mut().take(len).skip(i + 1) {
                *d = 0;
            }
            return true;
        }
    }
    true
}

/// Port of `_sys_strncasecmp`.  Case-insensitive compare honouring the
/// PS3 ctype table semantics.
#[must_use]
pub fn strncasecmp(a: &[u8], b: &[u8], n: u32) -> i32 {
    for i in 0..n as usize {
        let ca = tolower(a.get(i).copied().unwrap_or(0) as i32);
        let cb = tolower(b.get(i).copied().unwrap_or(0) as i32);
        if ca < cb { return -1; }
        if ca > cb { return 1; }
        if ca == 0 { break; }
    }
    0
}

/// Port of `_sys_strrchr`.  Returns the offset of the last `ch` byte
/// before the NUL terminator, or `None`.
#[must_use]
pub fn strrchr(s: &[u8], ch: u8) -> Option<usize> {
    let mut res = None;
    for (i, &b) in s.iter().enumerate() {
        if b == ch { res = Some(i); }
        if b == 0  { break; }
    }
    res
}

// =====================================================================
// `_sys_free` — unused for Rust tests but included for surface-count parity.
// =====================================================================

/// Port of `_sys_free`.  The firmware delegates to `vm::dealloc` and
/// unconditionally returns `CELL_OK`.
///
/// # Errors
/// Never returns an error in the C++ port; kept as `Result` so the
/// wrapper matches higher-layer signatures.
pub fn sys_free(_addr: u32) -> Result<(), CellError> { Ok(()) }

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- memcpy/memset/memmove/memcmp --------------------------------

    #[test]
    fn memcpy_copies_exact_bytes() {
        let mut dst = [0u8; 5];
        let src = [1, 2, 3, 4, 5];
        memcpy(&mut dst, &src, 5);
        assert_eq!(dst, src);
    }

    #[test]
    fn memcpy_partial_size() {
        let mut dst = [0u8; 5];
        let src = [1, 2, 3, 4, 5];
        memcpy(&mut dst, &src, 3);
        assert_eq!(dst, [1, 2, 3, 0, 0]);
    }

    #[test]
    fn memset_writes_low_byte_only() {
        let mut buf = [0u8; 4];
        memset(&mut buf, 0x1234_56AA_u32 as i32, 4);
        assert_eq!(buf, [0xAA; 4]);
    }

    #[test]
    fn memmove_overlap_forward() {
        let mut buf = [1u8, 2, 3, 4, 5, 0, 0, 0];
        memmove(&mut buf, 0, 2, 5);
        assert_eq!(&buf[2..7], &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn memmove_overlap_backward() {
        let mut buf = [0u8, 0, 1, 2, 3, 4, 5];
        memmove(&mut buf, 2, 0, 5);
        assert_eq!(&buf[..5], &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn memcmp_equal() {
        assert_eq!(memcmp(b"abc", b"abc", 3), 0);
    }

    #[test]
    fn memcmp_less_and_greater() {
        assert_eq!(memcmp(b"abc", b"abd", 3), -1);
        assert_eq!(memcmp(b"abd", b"abc", 3), 1);
    }

    #[test]
    fn memcmp_u32_returns_as_u32() {
        // -1 as u32 = 0xFFFF_FFFF
        assert_eq!(memcmp_u32(b"abc", b"abd", 3), 0xFFFF_FFFF);
        assert_eq!(memcmp_u32(b"abd", b"abc", 3), 1);
    }

    // ---- ctype table -------------------------------------------------

    #[test]
    fn ctype_table_length_matches_cpp() {
        assert_eq!(CTYPE_TABLE.len(), 129);
    }

    #[test]
    fn ctype_table_sentinel_is_zero() {
        assert_eq!(CTYPE_TABLE[0], 0); // ch = -1
    }

    #[test]
    fn ctype_table_digit_range_is_04() {
        // Digits '0'..='9' (0x30..=0x39) = 4 = digit flag
        for ch in 0x30..=0x39 {
            assert_eq!(CTYPE_TABLE[ch + 1], 4, "digit {ch:#x}");
        }
    }

    #[test]
    fn ctype_table_tab_is_0x408() {
        assert_eq!(CTYPE_TABLE[0x09 + 1], 0x408);
    }

    #[test]
    fn ctype_table_space_is_0x18() {
        // ' ' = 0x20 → table entry 0x18 (0x10 punct + 0x08 control-ish?)
        assert_eq!(CTYPE_TABLE[0x20 + 1], 0x18);
    }

    #[test]
    fn ctype_table_uppercase_a_is_0x41() {
        // 'A' = 0x41 → 0x41 (upper-bit 0x01 + hex-bit 0x40).
        // toupper() checks bit 0x02; tolower() checks bit 0x01.
        assert_eq!(CTYPE_TABLE[0x41 + 1], 0x41);
    }

    #[test]
    fn ctype_table_uppercase_g_is_0x01() {
        // 'G' = 0x47 → 0x01 (upper-bit only, not hex).
        assert_eq!(CTYPE_TABLE[0x47 + 1], 0x01);
    }

    #[test]
    fn ctype_table_lowercase_a_is_0x42() {
        // 'a' = 0x61 → 0x42 (lower-bit 0x02 + hex-bit 0x40).
        assert_eq!(CTYPE_TABLE[0x61 + 1], 0x42);
    }

    #[test]
    fn ctype_table_lowercase_g_is_0x02() {
        // 'g' = 0x67 → 0x02 (lower-bit only).
        assert_eq!(CTYPE_TABLE[0x67 + 1], 0x02);
    }

    #[test]
    fn ctype_table_delete_is_0x20() {
        // 0x7F = DEL → 0x20 (control)
        assert_eq!(CTYPE_TABLE[0x7F + 1], 0x20);
    }

    #[test]
    fn look_ctype_table_matches_raw() {
        assert_eq!(look_ctype_table(-1), 0);
        assert_eq!(look_ctype_table(0x30), 4);       // '0' → digit
        assert_eq!(look_ctype_table(0x41), 0x41);    // 'A' → upper + hex
        assert_eq!(look_ctype_table(0x61), 0x42);    // 'a' → lower + hex
        assert_eq!(look_ctype_table(0x20), 0x18);    // space
    }

    #[test]
    #[should_panic]
    fn look_ctype_table_panics_on_128() {
        look_ctype_table(128);
    }

    #[test]
    #[should_panic]
    fn look_ctype_table_panics_on_minus_2() {
        look_ctype_table(-2);
    }

    // ---- tolower / toupper ------------------------------------------

    #[test]
    fn tolower_converts_uppercase() {
        assert_eq!(tolower(b'A' as i32), b'a' as i32);
        assert_eq!(tolower(b'Z' as i32), b'z' as i32);
    }

    #[test]
    fn tolower_passes_through_others() {
        assert_eq!(tolower(b'a' as i32), b'a' as i32);
        assert_eq!(tolower(b'0' as i32), b'0' as i32);
        assert_eq!(tolower(b' ' as i32), b' ' as i32);
    }

    #[test]
    fn toupper_converts_lowercase() {
        assert_eq!(toupper(b'a' as i32), b'A' as i32);
        assert_eq!(toupper(b'z' as i32), b'Z' as i32);
    }

    #[test]
    fn toupper_passes_through_others() {
        assert_eq!(toupper(b'A' as i32), b'A' as i32);
        assert_eq!(toupper(b'0' as i32), b'0' as i32);
    }

    // ---- memchr -----------------------------------------------------

    #[test]
    fn memchr_finds_byte() {
        assert_eq!(memchr(Some(b"hello"), b'l', 5), Some(2));
    }

    #[test]
    fn memchr_not_found() {
        assert_eq!(memchr(Some(b"hello"), b'z', 5), None);
    }

    #[test]
    fn memchr_null_buf_returns_none() {
        assert_eq!(memchr(None, b'x', 5), None);
    }

    #[test]
    fn memchr_stops_at_size() {
        // 'l' is at offset 2, but size=2 → not found
        assert_eq!(memchr(Some(b"hello"), b'l', 2), None);
    }

    // ---- strlen / strcmp / strncmp ----------------------------------

    #[test]
    fn strlen_counts_until_nul() {
        assert_eq!(strlen(Some(b"hello\0world")), 5);
        assert_eq!(strlen(Some(b"\0")), 0);
    }

    #[test]
    fn strlen_null_ptr_returns_zero() {
        assert_eq!(strlen(None), 0);
    }

    #[test]
    fn strcmp_equal_returns_zero() {
        assert_eq!(strcmp(b"abc\0", b"abc\0"), 0);
    }

    #[test]
    fn strcmp_less_and_greater() {
        assert_eq!(strcmp(b"abc\0", b"abd\0"), -1);
        assert_eq!(strcmp(b"abd\0", b"abc\0"), 1);
        assert_eq!(strcmp(b"abc\0", b"abcd\0"), -1); // shorter is less
    }

    #[test]
    fn strncmp_bounded() {
        // Differ past the bound → still equal
        assert_eq!(strncmp(b"abc_xxx\0", b"abc_yyy\0", 3), 0);
        assert_eq!(strncmp(b"abc_xxx\0", b"abc_yyy\0", 5), -1);
    }

    // ---- strchr / strrchr -------------------------------------------

    #[test]
    fn strchr_finds_first() {
        assert_eq!(strchr(b"hello\0", b'l'), Some(2));
    }

    #[test]
    fn strchr_stops_at_nul() {
        assert_eq!(strchr(b"hi\0there\0", b'e'), None);
    }

    #[test]
    fn strrchr_finds_last() {
        assert_eq!(strrchr(b"hello\0", b'l'), Some(3));
    }

    #[test]
    fn strrchr_not_present() {
        assert_eq!(strrchr(b"hello\0", b'z'), None);
    }

    // ---- strcpy / strncpy -------------------------------------------

    #[test]
    fn strcpy_copies_with_nul() {
        let mut dst = [0u8; 8];
        strcpy(&mut dst, b"hi\0");
        assert_eq!(&dst[..3], b"hi\0");
    }

    #[test]
    fn strncpy_pads_with_nul_after_src() {
        let mut dst = [b'X'; 8];
        strncpy(Some(&mut dst), Some(b"hi\0"), 8);
        assert_eq!(&dst, b"hi\0\0\0\0\0\0");
    }

    #[test]
    fn strncpy_truncates_at_len() {
        let mut dst = [0u8; 8];
        strncpy(Some(&mut dst), Some(b"hello\0"), 3);
        assert_eq!(&dst[..3], b"hel");
    }

    #[test]
    fn strncpy_null_returns_false() {
        let mut dst = [0u8; 8];
        assert!(!strncpy(Some(&mut dst), None, 5));
        assert!(!strncpy(None, Some(b"x\0"), 5));
    }

    // ---- strcat / strncat -------------------------------------------

    #[test]
    fn strcat_appends() {
        let mut dst = [0u8; 16];
        dst[..4].copy_from_slice(b"foo\0");
        strcat(&mut dst, b"bar\0");
        assert_eq!(&dst[..7], b"foobar\0");
    }

    #[test]
    fn strncat_bounds_and_terminates() {
        let mut dst = [0u8; 16];
        dst[..4].copy_from_slice(b"foo\0");
        strncat(&mut dst, b"barbaz\0", 3);
        assert_eq!(&dst[..7], b"foobar\0");
    }

    // ---- strncasecmp ------------------------------------------------

    #[test]
    fn strncasecmp_ignores_case() {
        assert_eq!(strncasecmp(b"Hello\0", b"hello\0", 5), 0);
        assert_eq!(strncasecmp(b"HELLO\0", b"hello\0", 5), 0);
    }

    #[test]
    fn strncasecmp_detects_diff() {
        assert_eq!(strncasecmp(b"hella\0", b"hellz\0", 5), -1);
    }

    #[test]
    fn strncasecmp_bounded() {
        assert_eq!(strncasecmp(b"hello_XXX\0", b"HELLO_YYY\0", 5), 0);
    }

    // ---- sys_free ---------------------------------------------------

    #[test]
    fn sys_free_always_ok() {
        assert!(sys_free(0x1000).is_ok());
        assert!(sys_free(0).is_ok());
    }

    // ---- full smoke -------------------------------------------------

    #[test]
    fn full_libc_lifecycle_smoke() {
        // Allocate a 16-byte working buffer and exercise the full surface.
        let mut buf = [0u8; 16];

        // memset fills buf with 0xEE
        memset(&mut buf, 0xEE, 16);
        assert_eq!(buf, [0xEE; 16]);

        // strcpy overwrites with "hello"
        strcpy(&mut buf, b"hello\0");
        assert_eq!(&buf[..6], b"hello\0");

        // strlen reads it back
        assert_eq!(strlen(Some(&buf)), 5);

        // strcat appends "!"
        strcat(&mut buf, b"!\0");
        assert_eq!(&buf[..7], b"hello!\0");

        // strchr finds the '!'
        assert_eq!(strchr(&buf, b'!'), Some(5));

        // toupper the whole string byte-by-byte
        for b in buf.iter_mut() {
            if *b == 0 { break; }
            *b = toupper(*b as i32) as u8;
        }
        assert_eq!(&buf[..6], b"HELLO!");

        // strcmp against expected
        assert_eq!(strcmp(&buf, b"HELLO!\0"), 0);

        // memcpy backs up the first half
        let mut backup = [0u8; 8];
        memcpy(&mut backup, &buf, 8);
        assert_eq!(&backup[..6], b"HELLO!");

        // memcmp says they're equal for the first 7 bytes
        assert_eq!(memcmp(&buf, &backup, 7), 0);

        // sys_free never fails
        sys_free(0xDEAD_BEEF).unwrap();
    }
}
