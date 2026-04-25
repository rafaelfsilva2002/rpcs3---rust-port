//! `rpcs3-loader-tar` — Rust port of `rpcs3/Loader/TAR.cpp`.
//!
//! POSIX ustar archive reader. RPCS3 uses this to install PUP firmware
//! payloads: the installer streams individual tar members out of
//! `<firmware>.tar` and hands them to VFS. The C++ class additionally
//! handles serialization-backed streams for savestates, but the core
//! parsing logic is the same as any ustar reader.
//!
//! Frozen here:
//!
//! - `TARHeader` 512-byte record layout (header.h:9..22) with exact
//!   field offsets and sizes.
//! - `octal_text_to_u64(s)`: parse a 0-padded octal ASCII field. Ends at
//!   NUL or space. Returns `u64::MAX` on malformed input (cpp:59..71).
//! - Block layout: every tar record is a multiple of 512 bytes, so file
//!   payload advances by `((size + 511) / 512) * 512` (standard ustar).
//! - File-type character mapping: `'0'` or `'\0'` = regular file,
//!   `'5'` = directory, `'1'/'2'` = hard/symlink. We expose constants
//!   so callers don't need to guess.
//! - Prefix + name concatenation (ustar long-name handling): if
//!   `prefix` is non-empty, the full path is `prefix + "/" + name`.
//! - Pax "long link" records (`'L'`) are out of scope here — the cpp
//!   reader bails on them too.
//!
//! Actual filesystem I/O and the serialization stream variant live on
//! top of this crate; here we provide deterministic header parsing and
//! length computation so tests cover the bit-exact surface.

use core::mem::size_of;

/// POSIX ustar block size (all records are 512-byte aligned).
pub const BLOCK_SIZE: usize = 512;

/// `TARHeader` layout from `TAR.h:7..22`. 512 bytes total, verified with
/// a compile-time assertion.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct TarHeader {
    pub name: [u8; 100],
    pub dontcare: [u8; 24],
    pub size: [u8; 12],
    pub mtime: [u8; 12],
    pub chksum: [u8; 8],
    pub filetype: u8,
    pub linkname: [u8; 100],
    pub magic: [u8; 6],
    pub dontcare2: [u8; 82],
    pub prefix: [u8; 155],
    pub padding: [u8; 12],
}

const _: () = assert!(size_of::<TarHeader>() == BLOCK_SIZE);

impl Default for TarHeader {
    fn default() -> Self {
        Self {
            name: [0; 100],
            dontcare: [0; 24],
            size: [0; 12],
            mtime: [0; 12],
            chksum: [0; 8],
            filetype: 0,
            linkname: [0; 100],
            magic: [0; 6],
            dontcare2: [0; 82],
            prefix: [0; 155],
            padding: [0; 12],
        }
    }
}

/// Sentinel returned by `octal_text_to_u64` for malformed input (cpp uses
/// `u64::MAX` via `umax`).
pub const MALFORMED_OCTAL: u64 = u64::MAX;

/// File-type chars recognized by the cpp reader.
pub const FILETYPE_REGULAR_FILE: u8 = b'0';
pub const FILETYPE_REGULAR_FILE_ALT: u8 = 0; // NUL means "implicit regular"
pub const FILETYPE_HARDLINK: u8 = b'1';
pub const FILETYPE_SYMLINK: u8 = b'2';
pub const FILETYPE_DIRECTORY: u8 = b'5';

/// ustar magic bytes "ustar\0" (6 B) at offset 257 of the header.
pub const USTAR_MAGIC: [u8; 6] = *b"ustar\0";

/// Parse a NUL/space-terminated octal ASCII string into a u64.
///
/// Mirrors cpp:59..71. A pointer ending inside the buffer at a character
/// other than NUL or space (i.e. any stray non-octal digit) poisons the
/// result to `u64::MAX`.
#[must_use]
pub fn octal_text_to_u64(data: &[u8]) -> u64 {
    if data.is_empty() {
        return MALFORMED_OCTAL;
    }

    let mut value: u64 = 0;
    let mut consumed = 0usize;
    let mut saw_digit = false;

    for &b in data {
        match b {
            b'0'..=b'7' => {
                value = match value.checked_mul(8) {
                    Some(v) => v,
                    None => return MALFORMED_OCTAL,
                };
                value = match value.checked_add(u64::from(b - b'0')) {
                    Some(v) => v,
                    None => return MALFORMED_OCTAL,
                };
                saw_digit = true;
                consumed += 1;
            }
            b'\0' | b' ' => break,
            _ => return MALFORMED_OCTAL,
        }
    }

    if !saw_digit {
        return MALFORMED_OCTAL;
    }

    // If we consumed the entire slice without hitting NUL/space, cpp
    // treats it as malformed (cpp:65 `ptr == data.end() || ...`).
    if consumed == data.len() {
        return MALFORMED_OCTAL;
    }
    value
}

/// Round `size` up to the nearest multiple of 512 bytes (the tar block
/// alignment). Returns the total archive bytes occupied by a file's
/// payload (header excluded).
#[must_use]
pub const fn block_aligned_size(size: u64) -> u64 {
    size.div_ceil(BLOCK_SIZE as u64) * BLOCK_SIZE as u64
}

/// Extract a NUL-terminated C string from a fixed-size header field.
pub fn read_cstr(field: &[u8]) -> &[u8] {
    match field.iter().position(|&b| b == 0) {
        Some(n) => &field[..n],
        None => field,
    }
}

/// Compose full path from `prefix` + `name` per ustar long-name handling.
/// If `prefix` is non-empty, joins with `/`.
#[must_use]
pub fn full_name(header: &TarHeader) -> String {
    let name = std::str::from_utf8(read_cstr(&header.name)).unwrap_or("");
    let prefix = std::str::from_utf8(read_cstr(&header.prefix)).unwrap_or("");
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

/// Parse the `size` field of a header into a byte count; returns
/// `MALFORMED_OCTAL` if the field is garbage.
#[must_use]
pub fn header_size_bytes(header: &TarHeader) -> u64 {
    octal_text_to_u64(&header.size)
}

/// Classify a header's file-type byte.
#[must_use]
pub const fn header_filetype(header: &TarHeader) -> FileType {
    match header.filetype {
        FILETYPE_REGULAR_FILE | FILETYPE_REGULAR_FILE_ALT => FileType::Regular,
        FILETYPE_HARDLINK => FileType::Hardlink,
        FILETYPE_SYMLINK => FileType::Symlink,
        FILETYPE_DIRECTORY => FileType::Directory,
        other => FileType::Other(other),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    Regular,
    Hardlink,
    Symlink,
    Directory,
    Other(u8),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tar_header_is_512_bytes() {
        assert_eq!(size_of::<TarHeader>(), 512);
        assert_eq!(size_of::<TarHeader>(), BLOCK_SIZE);
    }

    #[test]
    fn octal_parse_basic() {
        // "0000755\0" (755 octal = 493 decimal, typical mode).
        let data = b"0000755\0";
        assert_eq!(octal_text_to_u64(data), 0o000_0755);
        assert_eq!(octal_text_to_u64(data), 493);
    }

    #[test]
    fn octal_parse_terminated_by_space() {
        let data = b"0001234 ";
        assert_eq!(octal_text_to_u64(data), 0o0001234);
    }

    #[test]
    fn octal_malformed_no_terminator() {
        // Consumed the whole slice without NUL/space — cpp poisons it.
        let data = b"12345";
        assert_eq!(octal_text_to_u64(data), MALFORMED_OCTAL);
    }

    #[test]
    fn octal_malformed_non_digit() {
        let data = b"1289\0"; // '8' and '9' are not octal; '8' → malformed.
        assert_eq!(octal_text_to_u64(data), MALFORMED_OCTAL);
    }

    #[test]
    fn octal_malformed_empty() {
        assert_eq!(octal_text_to_u64(b""), MALFORMED_OCTAL);
        assert_eq!(octal_text_to_u64(b"\0"), MALFORMED_OCTAL);
    }

    #[test]
    fn octal_small_field_with_space_padding() {
        // Typical chksum field: "006511 \0" (6-digit octal + space).
        let data = b"006511 \0";
        assert_eq!(octal_text_to_u64(data), 0o006511);
    }

    #[test]
    fn block_aligned_size_rounds_up() {
        assert_eq!(block_aligned_size(0), 0);
        assert_eq!(block_aligned_size(1), 512);
        assert_eq!(block_aligned_size(511), 512);
        assert_eq!(block_aligned_size(512), 512);
        assert_eq!(block_aligned_size(513), 1024);
        assert_eq!(block_aligned_size(1024), 1024);
        assert_eq!(block_aligned_size(1025), 1536);
    }

    #[test]
    fn read_cstr_strips_at_first_nul() {
        assert_eq!(read_cstr(b"hello\0world\0"), b"hello");
        assert_eq!(read_cstr(b"no-nul"), b"no-nul");
        assert_eq!(read_cstr(b"\0"), b"");
    }

    #[test]
    fn full_name_uses_prefix_when_present() {
        let mut h = TarHeader::default();
        h.name[..4].copy_from_slice(b"file");
        h.prefix[..6].copy_from_slice(b"pre/fx");
        assert_eq!(full_name(&h), "pre/fx/file");
    }

    #[test]
    fn full_name_bare_when_prefix_empty() {
        let mut h = TarHeader::default();
        h.name[..8].copy_from_slice(b"toplevel");
        assert_eq!(full_name(&h), "toplevel");
    }

    #[test]
    fn header_filetype_classification() {
        let mut h = TarHeader::default();
        h.filetype = b'0';
        assert_eq!(header_filetype(&h), FileType::Regular);
        h.filetype = 0;
        assert_eq!(header_filetype(&h), FileType::Regular);
        h.filetype = b'5';
        assert_eq!(header_filetype(&h), FileType::Directory);
        h.filetype = b'1';
        assert_eq!(header_filetype(&h), FileType::Hardlink);
        h.filetype = b'2';
        assert_eq!(header_filetype(&h), FileType::Symlink);
        h.filetype = b'x';
        assert_eq!(header_filetype(&h), FileType::Other(b'x'));
    }

    #[test]
    fn header_size_parses_octal_field() {
        let mut h = TarHeader::default();
        h.size[..8].copy_from_slice(b"0000012\0"); // 10 decimal bytes.
        assert_eq!(header_size_bytes(&h), 10);
    }

    #[test]
    fn ustar_magic_bytes() {
        assert_eq!(USTAR_MAGIC, *b"ustar\0");
        assert_eq!(USTAR_MAGIC.len(), 6);
    }
}
