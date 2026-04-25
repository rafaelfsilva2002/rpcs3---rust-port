//! `rpcs3-loader-pkg` — PKG header parsing.
//!
//! Mirrors the header-parsing portion of `rpcs3/Crypto/unpkg.{h,cpp}`.
//!
//! ## Scope
//!
//! * Parse `PKGHeader` (192 bytes, PS3 format).
//! * Identify release type, platform type, content type.
//! * Enumerate decrypted `PKGEntry` structs when the caller has the
//!   plaintext entry table.
//!
//! ## Out of scope (future waves)
//!
//! * Decryption — needs AES-CTR with per-PKG key derivation and HMAC
//!   validation. Will depend on `rpcs3-crypto`.
//! * File extraction to disk.
//! * Extended header (PKGExtHeader) for PSP/PSVita packages.

// ---------------------------------------------------------------------
// Constants (from rpcs3/Crypto/unpkg.h)
// ---------------------------------------------------------------------

/// Magic of the PKG header: `\x7fPKG` — read as u32 little-endian.
pub const PKG_MAGIC: u32 = 0x7F50_4B47;

pub const PKG_HEADER_SIZE: usize = 0xC0;
pub const PKG_HEADER_SIZE_EXT: usize = 0x280;

// pkg_type (release type)
pub const PKG_RELEASE_TYPE_RELEASE: u16 = 0x8000;
pub const PKG_RELEASE_TYPE_DEBUG: u16 = 0x0000;

// pkg_platform
pub const PKG_PLATFORM_PS3: u16 = 0x0001;
pub const PKG_PLATFORM_PSP_PSVITA: u16 = 0x0002;

// PKGEntry.type (low bits, ignoring flags)
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryType {
    Npdrm = 1,
    NpdrmEdat = 2,
    Regular = 3,
    Folder = 4,
    Unk0 = 5,
    Unk1 = 6,
    Sdat = 9,
    Other(u8) = 0xFF,
}

/// Entry type flag bits (ORed into the entry's u32 `type`).
pub const PKG_ENTRY_FLAG_OVERWRITE: u32 = 0x8000_0000;
pub const PKG_ENTRY_FLAG_PSP: u32 = 0x1000_0000;

// Content types observed in RPCS3 (unpkg.h:45-77)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ContentType {
    Unknown1 = 0x01,
    Unknown2 = 0x02,
    Unknown3 = 0x03,
    GameData = 0x04,
    GameExec = 0x05,
    Ps1Emu = 0x06,
    PcEngine = 0x07,
    Unknown4 = 0x08,
    Theme = 0x09,
    Widget = 0x0A,
    License = 0x0B,
    VshModule = 0x0C,
    PsnAvatar = 0x0D,
    PspGo = 0x0E,
    Minis = 0x0F,
    NeoGeo = 0x10,
    Vmc = 0x11,
    Ps2Classic = 0x12,
    Unknown5 = 0x13,
    PspRemastered = 0x14,
    Psp2Gd = 0x15,
    Psp2Ac = 0x16,
    Psp2La = 0x17,
    Psm1 = 0x18,
    Wt = 0x19,
    Unknown6 = 0x1A,
    Unknown7 = 0x1B,
    Unknown8 = 0x1C,
    Psm2 = 0x1D,
    Unknown9 = 0x1E,
    Psp2Theme = 0x1F,
    Other(u32) = 0xFFFF_FFFF,
}

impl ContentType {
    #[must_use]
    pub fn from_u32(v: u32) -> Self {
        match v {
            0x01 => Self::Unknown1,
            0x02 => Self::Unknown2,
            0x03 => Self::Unknown3,
            0x04 => Self::GameData,
            0x05 => Self::GameExec,
            0x06 => Self::Ps1Emu,
            0x07 => Self::PcEngine,
            0x08 => Self::Unknown4,
            0x09 => Self::Theme,
            0x0A => Self::Widget,
            0x0B => Self::License,
            0x0C => Self::VshModule,
            0x0D => Self::PsnAvatar,
            0x0E => Self::PspGo,
            0x0F => Self::Minis,
            0x10 => Self::NeoGeo,
            0x11 => Self::Vmc,
            0x12 => Self::Ps2Classic,
            0x13 => Self::Unknown5,
            0x14 => Self::PspRemastered,
            0x15 => Self::Psp2Gd,
            0x16 => Self::Psp2Ac,
            0x17 => Self::Psp2La,
            0x18 => Self::Psm1,
            0x19 => Self::Wt,
            0x1A => Self::Unknown6,
            0x1B => Self::Unknown7,
            0x1C => Self::Unknown8,
            0x1D => Self::Psm2,
            0x1E => Self::Unknown9,
            0x1F => Self::Psp2Theme,
            _ => Self::Other(v),
        }
    }
}

// ---------------------------------------------------------------------
// PKGHeader — 192 bytes (0xC0) total
// ---------------------------------------------------------------------

/// Parsed PKG header (PS3 variant). Mirrors `struct PKGHeader` in
/// `rpcs3/Crypto/unpkg.h:81-97`.
///
/// Note: `pkg_magic` is read as little-endian (per the `le_t` in C++);
/// `klicensee` is treated as an opaque 16-byte nonce (we don't decrypt
/// in this crate so endianness is immaterial).
#[derive(Debug, Clone)]
pub struct PkgHeader {
    pub pkg_magic: u32,
    pub pkg_type: u16,        // release type
    pub pkg_platform: u16,
    pub meta_offset: u32,
    pub meta_count: u32,
    pub meta_size: u32,
    pub file_count: u32,
    pub pkg_size: u64,
    pub data_offset: u64,
    pub data_size: u64,
    pub title_id: String,     // 48 bytes, NUL-terminated
    pub qa_digest: [u8; 16],  // 2× u64 BE
    pub klicensee: [u8; 16],  // 128-bit nonce
}

impl PkgHeader {
    #[must_use]
    pub fn is_release(&self) -> bool {
        self.pkg_type == PKG_RELEASE_TYPE_RELEASE
    }
    #[must_use]
    pub fn is_debug(&self) -> bool {
        self.pkg_type == PKG_RELEASE_TYPE_DEBUG
    }
    #[must_use]
    pub fn is_ps3(&self) -> bool {
        self.pkg_platform == PKG_PLATFORM_PS3
    }
    #[must_use]
    pub fn is_psp_or_psvita(&self) -> bool {
        self.pkg_platform == PKG_PLATFORM_PSP_PSVITA
    }
}

// ---------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    TooShort,
    BadMagic,
    /// `data_offset + data_size > pkg_size` or similar.
    CorruptOffsets,
    /// `title_id` bytes are not valid ASCII.
    BadTitleId,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::TooShort => f.write_str("input too short for PKG header"),
            Error::BadMagic => f.write_str("wrong PKG magic"),
            Error::CorruptOffsets => f.write_str("PKG offsets inconsistent with size"),
            Error::BadTitleId => f.write_str("title_id contains non-ASCII bytes"),
        }
    }
}

impl std::error::Error for Error {}

// ---------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------

fn read_u16_be(buf: &[u8], at: usize) -> u16 {
    u16::from_be_bytes(buf[at..at + 2].try_into().unwrap())
}

fn read_u32_be(buf: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(buf[at..at + 4].try_into().unwrap())
}

fn read_u32_le(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(buf[at..at + 4].try_into().unwrap())
}

fn read_u64_be(buf: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(buf[at..at + 8].try_into().unwrap())
}

/// Parse a PKG header from the first 192 bytes of `bytes`.
pub fn parse_header(bytes: &[u8]) -> Result<PkgHeader, Error> {
    if bytes.len() < PKG_HEADER_SIZE {
        return Err(Error::TooShort);
    }

    let pkg_magic = read_u32_le(bytes, 0);
    if pkg_magic != PKG_MAGIC {
        return Err(Error::BadMagic);
    }

    let pkg_type = read_u16_be(bytes, 4);
    let pkg_platform = read_u16_be(bytes, 6);
    let meta_offset = read_u32_be(bytes, 8);
    let meta_count = read_u32_be(bytes, 12);
    let meta_size = read_u32_be(bytes, 16);
    let file_count = read_u32_be(bytes, 20);
    let pkg_size = read_u64_be(bytes, 24);
    let data_offset = read_u64_be(bytes, 32);
    let data_size = read_u64_be(bytes, 40);

    // title_id at offset 48, 48 bytes. NUL-terminated ASCII.
    let title_raw = &bytes[48..48 + 48];
    let title_nul = title_raw.iter().position(|&b| b == 0).unwrap_or(title_raw.len());
    let title_id = std::str::from_utf8(&title_raw[..title_nul])
        .map_err(|_| Error::BadTitleId)?
        .to_owned();
    if !title_id.is_ascii() {
        return Err(Error::BadTitleId);
    }

    let mut qa_digest = [0u8; 16];
    qa_digest.copy_from_slice(&bytes[96..112]);
    let mut klicensee = [0u8; 16];
    klicensee.copy_from_slice(&bytes[112..128]);

    // Offset sanity — mirror unpkg.cpp validation loosely.
    if data_offset.saturating_add(data_size) > pkg_size && pkg_size > 0 {
        return Err(Error::CorruptOffsets);
    }

    Ok(PkgHeader {
        pkg_magic,
        pkg_type,
        pkg_platform,
        meta_offset,
        meta_count,
        meta_size,
        file_count,
        pkg_size,
        data_offset,
        data_size,
        title_id,
        qa_digest,
        klicensee,
    })
}

// ---------------------------------------------------------------------
// PKGEntry — 32 bytes (big-endian)
// ---------------------------------------------------------------------

/// A PKG file-entry record, decrypted by the caller.
/// Mirrors `struct PKGEntry` in `rpcs3/Crypto/unpkg.h:115`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PkgEntry {
    pub name_offset: u32,
    pub name_size: u32,
    pub file_offset: u64,
    pub file_size: u64,
    pub raw_type: u32,
    pub pad: u32,
}

pub const PKG_ENTRY_SIZE: usize = 32;

impl PkgEntry {
    /// Entry's logical type, discarding flag bits.
    #[must_use]
    pub fn entry_type(&self) -> EntryType {
        match self.raw_type & 0xFF {
            1 => EntryType::Npdrm,
            2 => EntryType::NpdrmEdat,
            3 => EntryType::Regular,
            4 => EntryType::Folder,
            5 => EntryType::Unk0,
            6 => EntryType::Unk1,
            9 => EntryType::Sdat,
            other => EntryType::Other(other as u8),
        }
    }

    #[must_use]
    pub fn is_overwrite(&self) -> bool {
        self.raw_type & PKG_ENTRY_FLAG_OVERWRITE != 0
    }

    #[must_use]
    pub fn is_psp(&self) -> bool {
        self.raw_type & PKG_ENTRY_FLAG_PSP != 0
    }
}

/// Parse a single `PKGEntry` from the given 32-byte slice.
pub fn parse_entry(bytes: &[u8]) -> Result<PkgEntry, Error> {
    if bytes.len() < PKG_ENTRY_SIZE {
        return Err(Error::TooShort);
    }
    Ok(PkgEntry {
        name_offset: read_u32_be(bytes, 0),
        name_size: read_u32_be(bytes, 4),
        file_offset: read_u64_be(bytes, 8),
        file_size: read_u64_be(bytes, 16),
        raw_type: read_u32_be(bytes, 24),
        pad: read_u32_be(bytes, 28),
    })
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid PKG header (192 bytes) for a PS3 release.
    fn build_header(
        pkg_size: u64,
        data_offset: u64,
        data_size: u64,
        title_id: &str,
    ) -> Vec<u8> {
        let mut bytes = vec![0u8; PKG_HEADER_SIZE];
        bytes[0..4].copy_from_slice(&PKG_MAGIC.to_le_bytes());
        bytes[4..6].copy_from_slice(&PKG_RELEASE_TYPE_RELEASE.to_be_bytes());
        bytes[6..8].copy_from_slice(&PKG_PLATFORM_PS3.to_be_bytes());
        bytes[8..12].copy_from_slice(&(PKG_HEADER_SIZE as u32).to_be_bytes()); // meta_offset
        bytes[12..16].copy_from_slice(&0u32.to_be_bytes()); // meta_count
        bytes[16..20].copy_from_slice(&0u32.to_be_bytes()); // meta_size
        bytes[20..24].copy_from_slice(&1u32.to_be_bytes()); // file_count
        bytes[24..32].copy_from_slice(&pkg_size.to_be_bytes());
        bytes[32..40].copy_from_slice(&data_offset.to_be_bytes());
        bytes[40..48].copy_from_slice(&data_size.to_be_bytes());
        // title_id — 48 bytes, NUL-padded
        let tbytes = title_id.as_bytes();
        bytes[48..48 + tbytes.len()].copy_from_slice(tbytes);
        // qa_digest and klicensee left as zeros
        bytes
    }

    #[test]
    fn header_rejects_short_input() {
        assert_eq!(parse_header(&[0u8; 32]).unwrap_err(), Error::TooShort);
    }

    #[test]
    fn header_rejects_wrong_magic() {
        let mut bytes = build_header(0x1000, 0x400, 0xC00, "NPUB12345_00");
        bytes[0..4].copy_from_slice(b"XXXX");
        assert_eq!(parse_header(&bytes).unwrap_err(), Error::BadMagic);
    }

    #[test]
    fn header_parses_minimal_release() {
        // 1 MB package: header prefix + encrypted data region that ends
        // exactly at pkg_size. data_size = pkg_size - data_offset.
        let bytes = build_header(0x10_0000, 0x400, 0x0F_FC00, "NPUB12345_00");
        let h = parse_header(&bytes).unwrap();
        assert_eq!(h.pkg_magic, PKG_MAGIC);
        assert!(h.is_release());
        assert!(h.is_ps3());
        assert!(!h.is_debug());
        assert_eq!(h.file_count, 1);
        assert_eq!(h.pkg_size, 0x10_0000);
        assert_eq!(h.data_offset, 0x400);
        assert_eq!(h.data_size, 0x0F_FC00);
        assert_eq!(h.title_id, "NPUB12345_00");
        assert_eq!(h.qa_digest, [0u8; 16]);
    }

    #[test]
    fn header_detects_corrupt_offsets() {
        // data_offset (0x800) + data_size (0x100) > pkg_size (0x800)
        let bytes = build_header(0x800, 0x800, 0x100, "X");
        assert_eq!(parse_header(&bytes).unwrap_err(), Error::CorruptOffsets);
    }

    #[test]
    fn header_detects_non_ascii_title_id() {
        let mut bytes = build_header(0x1000, 0x400, 0xC00, "");
        bytes[48] = 0xFF; // invalid ASCII
        bytes[49] = 0xFE;
        assert_eq!(parse_header(&bytes).unwrap_err(), Error::BadTitleId);
    }

    #[test]
    fn header_pkg_size_zero_skips_offset_check() {
        // When pkg_size is 0 we skip the corruption check (see parse_header).
        let bytes = build_header(0, 0x1_0000_0000, 0x1000, "T");
        let h = parse_header(&bytes).unwrap();
        assert_eq!(h.pkg_size, 0);
    }

    #[test]
    fn debug_release_type_detected() {
        let mut bytes = build_header(0x1000, 0x400, 0xC00, "T");
        bytes[4..6].copy_from_slice(&PKG_RELEASE_TYPE_DEBUG.to_be_bytes());
        let h = parse_header(&bytes).unwrap();
        assert!(h.is_debug());
        assert!(!h.is_release());
    }

    #[test]
    fn psvita_platform_detected() {
        let mut bytes = build_header(0x1000, 0x400, 0xC00, "T");
        bytes[6..8].copy_from_slice(&PKG_PLATFORM_PSP_PSVITA.to_be_bytes());
        let h = parse_header(&bytes).unwrap();
        assert!(h.is_psp_or_psvita());
        assert!(!h.is_ps3());
    }

    // -- PkgEntry -------------------------------------------------

    fn build_entry(name_offset: u32, name_size: u32, file_size: u64, type_raw: u32) -> Vec<u8> {
        let mut bytes = vec![0u8; PKG_ENTRY_SIZE];
        bytes[0..4].copy_from_slice(&name_offset.to_be_bytes());
        bytes[4..8].copy_from_slice(&name_size.to_be_bytes());
        bytes[8..16].copy_from_slice(&0u64.to_be_bytes()); // file_offset
        bytes[16..24].copy_from_slice(&file_size.to_be_bytes());
        bytes[24..28].copy_from_slice(&type_raw.to_be_bytes());
        bytes[28..32].copy_from_slice(&0u32.to_be_bytes());
        bytes
    }

    #[test]
    fn entry_parses_regular_file() {
        let bytes = build_entry(0, 20, 0x1000, 3);
        let e = parse_entry(&bytes).unwrap();
        assert_eq!(e.entry_type(), EntryType::Regular);
        assert_eq!(e.file_size, 0x1000);
        assert_eq!(e.name_size, 20);
        assert!(!e.is_overwrite());
        assert!(!e.is_psp());
    }

    #[test]
    fn entry_detects_overwrite_flag() {
        let bytes = build_entry(0, 10, 0x500, 3 | PKG_ENTRY_FLAG_OVERWRITE);
        let e = parse_entry(&bytes).unwrap();
        assert_eq!(e.entry_type(), EntryType::Regular);
        assert!(e.is_overwrite());
    }

    #[test]
    fn entry_detects_psp_flag() {
        let bytes = build_entry(0, 10, 0x500, 3 | PKG_ENTRY_FLAG_PSP);
        let e = parse_entry(&bytes).unwrap();
        assert!(e.is_psp());
    }

    #[test]
    fn entry_parses_folder_type() {
        let bytes = build_entry(0, 5, 0, 4);
        let e = parse_entry(&bytes).unwrap();
        assert_eq!(e.entry_type(), EntryType::Folder);
    }

    #[test]
    fn entry_parses_npdrm_type() {
        let bytes = build_entry(0, 12, 0x2000, 1);
        let e = parse_entry(&bytes).unwrap();
        assert_eq!(e.entry_type(), EntryType::Npdrm);
    }

    #[test]
    fn entry_rejects_short_input() {
        assert!(parse_entry(&[0u8; 16]).is_err());
    }

    // -- ContentType enum --------------------------------------------

    #[test]
    fn content_type_known_values() {
        assert_eq!(ContentType::from_u32(0x04), ContentType::GameData);
        assert_eq!(ContentType::from_u32(0x05), ContentType::GameExec);
        assert_eq!(ContentType::from_u32(0x06), ContentType::Ps1Emu);
        assert_eq!(ContentType::from_u32(0x09), ContentType::Theme);
        assert_eq!(ContentType::from_u32(0x12), ContentType::Ps2Classic);
        assert_eq!(ContentType::from_u32(0x1F), ContentType::Psp2Theme);
    }

    #[test]
    fn content_type_unknown_is_other() {
        assert_eq!(ContentType::from_u32(0x99), ContentType::Other(0x99));
    }
}
