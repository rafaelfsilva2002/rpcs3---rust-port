//! `rpcs3-loader-iso` — Rust port of `rpcs3/Loader/ISO.cpp`.
//!
//! PS3 3k3y / Redump ISO reader. The C++ loader parses the 4-byte region
//! table at the start of the image, distinguishes encrypted vs plaintext
//! sectors (even index = plaintext, odd = encrypted), and wraps the
//! AES-128-CBC decrypt path with LBA-derived IVs.
//!
//! What we freeze here is the parser side that doesn't need an AES
//! implementation — magic detection, sector math, IV derivation, region
//! table parsing — so tests cover the bit-exact surface without having
//! to link a crypto library.
//!
//! Frozen:
//!
//! - `ISO_SECTOR_SIZE = 2048` (cpp:16).
//! - ISO 9660 magic `"CD001"` at file offset 32768 + 1 (cpp:42..49).
//! - `char_arr_BE_to_uint` — 4-byte big-endian to `u32` (cpp:53..56).
//! - `reset_iv(iv, lba)` — zero first 12 bytes; last 4 are the LBA
//!   big-endian (cpp:58..67).
//! - Sector-count math for `decrypt_data(offset, size)`: the number of
//!   sectors that a `[offset, offset + size)` range touches (cpp:86..88).
//! - Region-table constraint `1 <= region_count <= 127` (cpp:202).
//! - "Even region index = plaintext, odd = encrypted" invariant (cpp:197).

pub const ISO_SECTOR_SIZE: u64 = 2048;

/// ISO 9660 Primary Volume Descriptor signature (cpp:46..49).
pub const ISO_9660_MAGIC: &[u8; 5] = b"CD001";

/// File offset where `CD001` lives (ISO 9660 volume descriptor 1) plus
/// the 1-byte type field (cpp:43..44). The magic is at
/// `MAGIC_BYTE_OFFSET = 32768 + 1 = 32769`.
pub const MAGIC_BYTE_OFFSET: u64 = 32768 + 1;

pub const MIN_ISO_FILE_SIZE: u64 = 32768 + 6;

/// Convert a 4-byte big-endian buffer to `u32` (cpp:53..56).
#[must_use]
pub const fn char_arr_be_to_uint(buf: &[u8; 4]) -> u32 {
    ((buf[0] as u32) << 24)
        | ((buf[1] as u32) << 16)
        | ((buf[2] as u32) << 8)
        | (buf[3] as u32)
}

/// Generate the AES-128-CBC IV used for a given LBA: 12 zero bytes
/// followed by the 4-byte big-endian LBA (cpp:58..67).
#[must_use]
pub fn reset_iv(lba: u32) -> [u8; 16] {
    let mut iv = [0u8; 16];
    iv[12] = ((lba & 0xFF00_0000) >> 24) as u8;
    iv[13] = ((lba & 0x00FF_0000) >> 16) as u8;
    iv[14] = ((lba & 0x0000_FF00) >> 8) as u8;
    iv[15] = (lba & 0x0000_00FF) as u8;
    iv
}

/// First-sector LBA for a byte `offset` (cpp:86).
#[must_use]
pub const fn first_sector_lba(offset: u64) -> u32 {
    (offset / ISO_SECTOR_SIZE) as u32
}

/// Number of sectors touched by the byte range `[offset, offset + size)`
/// (cpp:87). `size` must be > 0.
#[must_use]
pub const fn touched_sector_count(offset: u64, size: u64) -> u32 {
    let first = first_sector_lba(offset);
    let last = ((offset + size - 1) / ISO_SECTOR_SIZE) as u32;
    last - first + 1
}

/// Starting offset inside the first sector (cpp:88).
#[must_use]
pub const fn sector_offset(offset: u64) -> u64 {
    offset % ISO_SECTOR_SIZE
}

/// Check that the ISO magic bytes appear at the PVD offset.
/// Requires a buffer covering indices `[32769..=32773]`.
#[must_use]
pub fn has_iso_magic(buf: &[u8]) -> bool {
    if buf.len() < (MAGIC_BYTE_OFFSET + ISO_9660_MAGIC.len() as u64) as usize {
        return false;
    }
    &buf[MAGIC_BYTE_OFFSET as usize..MAGIC_BYTE_OFFSET as usize + 5] == ISO_9660_MAGIC
}

/// Region index parity per 3k3y layout (cpp:197). Even regions (including
/// region 0) are plaintext; odd regions are encrypted.
#[must_use]
pub const fn region_is_encrypted(region_index: u32) -> bool {
    region_index % 2 == 1
}

/// `region_count` constraint (cpp:202). Valid PS3 ISOs have 1..=127.
#[must_use]
pub const fn is_valid_region_count(count: u32) -> bool {
    count >= 1 && count <= 127
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sector_size_and_magic_offsets() {
        assert_eq!(ISO_SECTOR_SIZE, 2048);
        assert_eq!(MAGIC_BYTE_OFFSET, 32769);
        assert_eq!(MIN_ISO_FILE_SIZE, 32768 + 6);
        assert_eq!(ISO_9660_MAGIC, b"CD001");
    }

    #[test]
    fn char_arr_be_to_uint_standard_values() {
        assert_eq!(char_arr_be_to_uint(&[0, 0, 0, 0]), 0);
        assert_eq!(char_arr_be_to_uint(&[0, 0, 0, 1]), 1);
        assert_eq!(char_arr_be_to_uint(&[0x12, 0x34, 0x56, 0x78]), 0x1234_5678);
        assert_eq!(char_arr_be_to_uint(&[0xFF, 0xFF, 0xFF, 0xFF]), 0xFFFF_FFFF);
    }

    #[test]
    fn reset_iv_layout() {
        let iv = reset_iv(0x1234_5678);
        assert_eq!(&iv[..12], &[0u8; 12]);
        assert_eq!(&iv[12..16], &[0x12, 0x34, 0x56, 0x78]);
    }

    #[test]
    fn reset_iv_zero_lba() {
        assert_eq!(reset_iv(0), [0u8; 16]);
    }

    #[test]
    fn first_sector_lba_and_offsets() {
        assert_eq!(first_sector_lba(0), 0);
        assert_eq!(first_sector_lba(2047), 0);
        assert_eq!(first_sector_lba(2048), 1);
        assert_eq!(first_sector_lba(4096), 2);
        assert_eq!(sector_offset(2050), 2);
        assert_eq!(sector_offset(0), 0);
    }

    #[test]
    fn touched_sector_count_cases() {
        // Fits entirely in sector 0.
        assert_eq!(touched_sector_count(0, 2048), 1);
        assert_eq!(touched_sector_count(100, 500), 1);
        // Exactly two sectors.
        assert_eq!(touched_sector_count(0, 2049), 2);
        assert_eq!(touched_sector_count(2048, 2048), 1);
        // Spans three sectors (offset in the middle).
        assert_eq!(touched_sector_count(2000, 3000), 3);
        // One byte straddles two sectors.
        assert_eq!(touched_sector_count(2047, 2), 2);
    }

    #[test]
    fn has_iso_magic_detects_cd001() {
        let mut buf = vec![0u8; (MAGIC_BYTE_OFFSET + 5) as usize];
        buf[MAGIC_BYTE_OFFSET as usize..MAGIC_BYTE_OFFSET as usize + 5]
            .copy_from_slice(b"CD001");
        assert!(has_iso_magic(&buf));
    }

    #[test]
    fn has_iso_magic_rejects_wrong_or_short_buffer() {
        // Wrong magic.
        let mut buf = vec![0u8; (MAGIC_BYTE_OFFSET + 5) as usize];
        buf[MAGIC_BYTE_OFFSET as usize..MAGIC_BYTE_OFFSET as usize + 5]
            .copy_from_slice(b"XXXXX");
        assert!(!has_iso_magic(&buf));
        // Too short.
        assert!(!has_iso_magic(&[0u8; 100]));
    }

    #[test]
    fn region_parity_matches_cpp() {
        assert!(!region_is_encrypted(0));
        assert!(region_is_encrypted(1));
        assert!(!region_is_encrypted(2));
        assert!(region_is_encrypted(3));
        assert!(!region_is_encrypted(126));
        assert!(region_is_encrypted(127));
    }

    #[test]
    fn region_count_bounds_1_to_127() {
        assert!(!is_valid_region_count(0));
        assert!(is_valid_region_count(1));
        assert!(is_valid_region_count(64));
        assert!(is_valid_region_count(127));
        assert!(!is_valid_region_count(128));
        assert!(!is_valid_region_count(0xFFFF_FFFF));
    }
}
