//! `rpcs3-loader-trp` — Rust port of `rpcs3/Loader/TRP.cpp`.
//!
//! Reader for the PS3 Trophy (TRP) archive format. A TRP bundles the
//! trophy icons, XML descriptor and per-user progress into a single
//! big-endian container. RPCS3 mounts it during first launch of a game
//! to populate the trophy UI.
//!
//! Frozen here:
//!
//! - `TRPHeader` 64-byte layout (h:3..13). Magic word `0xDCA24D00` at
//!   offset 0 (BE).
//! - `TRPEntry` 64-byte records (32-byte name + BE u64 offset + BE u64
//!   size + BE u32 flags + 12-byte padding).
//! - Version-2 SHA1 verification contract: zero out the SHA1 field,
//!   re-hash the full file, compare against stored hash (cpp:117..138).
//! - `GetRequiredSpace` formula: `file_size - sizeof(header) -
//!   files_count * element_size` (cpp:160..166).
//! - `ContainsEntry` / `RemoveEntry` / `RenameEntry` — all reject names
//!   that would not fit in the 32-byte `name` field.
//!
//! Filesystem install flow (Install, copy-to-temp + atomic-rename) stays
//! in the frontend; we expose the pure parser + entry manipulation here.

use core::mem::size_of;

pub const TRP_MAGIC: u32 = 0xDCA2_4D00;
/// Byte offset inside `TRPHeader` where the 20-byte SHA1 lives
/// (header_bytes_before_sha1 = 4+4+8+4+4+4 = 28). Used when the caller
/// needs to zero it for verification.
pub const TRP_SHA1_OFFSET: usize = 28;
pub const TRP_SHA1_LEN: usize = 20;

/// Maximum length (INCLUSIVE of NUL) that fits in a TRPEntry name field.
/// `cpp uses >= sizeof(TRPEntry::name)` as the rejection condition.
pub const TRP_ENTRY_NAME_MAX: usize = 32;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrpHeader {
    /// Stored big-endian on disk; we keep it as host-endian here and
    /// require the caller to flip on read. Matches `be_t<u32>`.
    pub trp_magic: u32,
    pub trp_version: u32,
    pub trp_file_size: u64,
    pub trp_files_count: u32,
    pub trp_element_size: u32,
    pub trp_dev_flag: u32,
    pub sha1: [u8; 20],
    pub padding: [u8; 16],
}

impl Default for TrpHeader {
    fn default() -> Self {
        Self {
            trp_magic: 0,
            trp_version: 0,
            trp_file_size: 0,
            trp_files_count: 0,
            trp_element_size: 0,
            trp_dev_flag: 0,
            sha1: [0; 20],
            padding: [0; 16],
        }
    }
}

const _: () = assert!(size_of::<TrpHeader>() == 64);

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrpEntry {
    pub name: [u8; 32],
    pub offset: u64,
    pub size: u64,
    pub unknown: u32,
    pub padding: [u8; 12],
}

impl Default for TrpEntry {
    fn default() -> Self {
        Self {
            name: [0; 32],
            offset: 0,
            size: 0,
            unknown: 0,
            padding: [0; 12],
        }
    }
}

const _: () = assert!(size_of::<TrpEntry>() == 64);

impl TrpEntry {
    /// Fills `name` with a NUL-terminated ASCII string, truncating and
    /// padding with zeros. Returns `false` if `name` doesn't fit (cpp
    /// rejection condition for rename/remove/contains).
    pub fn set_name(&mut self, new_name: &str) -> bool {
        let bytes = new_name.as_bytes();
        if bytes.len() >= TRP_ENTRY_NAME_MAX {
            return false;
        }
        self.name = [0; 32];
        self.name[..bytes.len()].copy_from_slice(bytes);
        true
    }

    /// Returns the name as a &str (up to first NUL). Invalid UTF-8
    /// bytes collapse to `""` — TRP names are always ASCII in practice.
    #[must_use]
    pub fn name_str(&self) -> &str {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(self.name.len());
        core::str::from_utf8(&self.name[..end]).unwrap_or("")
    }
}

/// Verify a header's magic word. Returns false for any other value
/// (cpp:107..110).
#[must_use]
pub const fn is_valid_magic(magic: u32) -> bool {
    magic == TRP_MAGIC
}

/// Compute `GetRequiredSpace()` (cpp:160..166).
#[must_use]
pub const fn required_space(
    file_size: u64,
    files_count: u64,
    element_size: u64,
) -> u64 {
    let header_size = size_of::<TrpHeader>() as u64;
    let elements_total = files_count.saturating_mul(element_size);
    file_size
        .saturating_sub(header_size)
        .saturating_sub(elements_total)
}

/// Search entries for a name. Mirrors cpp:168..183: oversize names
/// return `false` without iterating.
#[must_use]
pub fn contains_entry(entries: &[TrpEntry], filename: &str) -> bool {
    if filename.len() >= TRP_ENTRY_NAME_MAX {
        return false;
    }
    entries.iter().any(|e| e.name_str() == filename)
}

/// Remove entries matching `filename`. Returns the number removed.
/// Oversize names are no-ops (cpp:185..204).
pub fn remove_entry(entries: &mut Vec<TrpEntry>, filename: &str) -> usize {
    if filename.len() >= TRP_ENTRY_NAME_MAX {
        return 0;
    }
    let before = entries.len();
    entries.retain(|e| e.name_str() != filename);
    before - entries.len()
}

/// Rename entries matching `oldname` to `newname`. Oversize names
/// (either side) cause a no-op. Returns how many entries were renamed.
pub fn rename_entry(entries: &mut [TrpEntry], oldname: &str, newname: &str) -> usize {
    if oldname.len() >= TRP_ENTRY_NAME_MAX || newname.len() >= TRP_ENTRY_NAME_MAX {
        return 0;
    }
    let mut count = 0usize;
    for e in entries.iter_mut() {
        if e.name_str() == oldname {
            if e.set_name(newname) {
                count += 1;
            }
        }
    }
    count
}

/// Produces a clone of `file_contents` with the SHA1 bytes zeroed out,
/// ready to feed into SHA1 for version-2 verification (cpp:129).
/// Requires `file_contents.len() >= TRP_SHA1_OFFSET + TRP_SHA1_LEN`.
pub fn prepare_for_sha1(file_contents: &[u8]) -> Option<Vec<u8>> {
    if file_contents.len() < TRP_SHA1_OFFSET + TRP_SHA1_LEN {
        return None;
    }
    let mut buf = file_contents.to_vec();
    for b in &mut buf[TRP_SHA1_OFFSET..TRP_SHA1_OFFSET + TRP_SHA1_LEN] {
        *b = 0;
    }
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_and_entry_sizes() {
        assert_eq!(size_of::<TrpHeader>(), 64);
        assert_eq!(size_of::<TrpEntry>(), 64);
    }

    #[test]
    fn magic_constant() {
        assert_eq!(TRP_MAGIC, 0xDCA2_4D00);
        assert!(is_valid_magic(0xDCA2_4D00));
        assert!(!is_valid_magic(0x1234_5678));
    }

    #[test]
    fn required_space_formula() {
        // file_size = 1000, 2 entries * 64 bytes each = 128 elements.
        // Header = 64. Required = 1000 - 64 - 128 = 808.
        assert_eq!(required_space(1000, 2, 64), 808);
        // Underflow-safe: saturating subtraction.
        assert_eq!(required_space(10, 100, 64), 0);
    }

    #[test]
    fn set_name_rejects_oversize() {
        let mut e = TrpEntry::default();
        // 31 chars fits (leaves space for NUL at index 31).
        let ok = "a".repeat(31);
        assert!(e.set_name(&ok));
        assert_eq!(e.name_str(), ok);
        // 32 chars is rejected (no room for NUL, matches cpp >= condition).
        let too_big = "a".repeat(32);
        assert!(!e.set_name(&too_big));
    }

    #[test]
    fn set_name_preserves_trailing_zeros() {
        let mut e = TrpEntry::default();
        e.set_name("TROP.SFM");
        assert_eq!(&e.name[..8], b"TROP.SFM");
        assert_eq!(e.name[8], 0);
        assert_eq!(e.name[31], 0);
    }

    #[test]
    fn name_str_stops_at_first_nul() {
        let mut e = TrpEntry::default();
        e.name[..4].copy_from_slice(b"FILE");
        // Rest is zero → name_str returns "FILE".
        assert_eq!(e.name_str(), "FILE");
    }

    #[test]
    fn contains_entry_matches_and_rejects_oversize() {
        let mut e = TrpEntry::default();
        e.set_name("ICON.PNG");
        let entries = vec![e];
        assert!(contains_entry(&entries, "ICON.PNG"));
        assert!(!contains_entry(&entries, "OTHER.PNG"));
        // Oversize query rejected before iteration.
        let huge = "x".repeat(32);
        assert!(!contains_entry(&entries, &huge));
    }

    #[test]
    fn remove_entry_deletes_all_matches() {
        let mut entries = Vec::new();
        let mut e1 = TrpEntry::default();
        e1.set_name("A");
        let mut e2 = TrpEntry::default();
        e2.set_name("B");
        let mut e3 = TrpEntry::default();
        e3.set_name("A");
        entries.push(e1);
        entries.push(e2);
        entries.push(e3);

        assert_eq!(remove_entry(&mut entries, "A"), 2);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name_str(), "B");
    }

    #[test]
    fn remove_entry_oversize_is_noop() {
        let mut entries = vec![TrpEntry::default()];
        let huge = "x".repeat(32);
        assert_eq!(remove_entry(&mut entries, &huge), 0);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn rename_entry_updates_matches() {
        let mut e = TrpEntry::default();
        e.set_name("OLD");
        let mut entries = vec![e];
        assert_eq!(rename_entry(&mut entries, "OLD", "NEW"), 1);
        assert_eq!(entries[0].name_str(), "NEW");
    }

    #[test]
    fn rename_entry_oversize_any_side_is_noop() {
        let mut e = TrpEntry::default();
        e.set_name("SHORT");
        let mut entries = vec![e];
        let huge = "x".repeat(32);
        assert_eq!(rename_entry(&mut entries, &huge, "NEW"), 0);
        assert_eq!(rename_entry(&mut entries, "SHORT", &huge), 0);
        assert_eq!(entries[0].name_str(), "SHORT");
    }

    #[test]
    fn prepare_for_sha1_zeros_the_sha1_field_only() {
        let mut file = vec![0xFFu8; 128];
        // Fill positions [28..=47] with known values — prepare_for_sha1
        // should zero exactly that range.
        for b in &mut file[28..48] {
            *b = 0xAA;
        }
        let prepared = prepare_for_sha1(&file).expect("big enough");
        assert_eq!(&prepared[0..28], &vec![0xFFu8; 28][..]);
        assert_eq!(&prepared[28..48], &[0u8; 20]);
        assert_eq!(&prepared[48..128], &vec![0xFFu8; 80][..]);
    }

    #[test]
    fn prepare_for_sha1_rejects_short_input() {
        assert!(prepare_for_sha1(&[0u8; 20]).is_none());
    }

    #[test]
    fn trp_sha1_offset_and_len_match_header() {
        // 4 (magic) + 4 (version) + 8 (file_size) + 4 (files_count) +
        // 4 (element_size) + 4 (dev_flag) = 28.
        assert_eq!(TRP_SHA1_OFFSET, 28);
        assert_eq!(TRP_SHA1_LEN, 20);
    }
}
