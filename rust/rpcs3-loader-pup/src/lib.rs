//! `rpcs3-loader-pup` — PS3 firmware PUP file parser.
//!
//! Mirrors `rpcs3/Loader/PUP.{h,cpp}`.
//!
//! ## File format (big-endian fields, magic is LE)
//!
//! ```text
//! Header (48 bytes):
//!   magic           u64 LE  = "SCEUF\0\0\0"
//!   package_version u64 BE
//!   image_version   u64 BE
//!   file_count      u64 BE
//!   header_length   u64 BE  (offset of first byte after file+hash tables)
//!   data_length     u64 BE  (size of the data region)
//!
//! FileEntry[file_count] (32 bytes each):
//!   entry_id    u64 BE
//!   data_offset u64 BE  (relative to PUP start)
//!   data_length u64 BE
//!   padding     u8[8]
//!
//! HashEntry[file_count] (32 bytes each):
//!   entry_id u64 BE
//!   hash     u8[20]   (SHA-1 of the file's bytes)
//!   padding  u8[4]
//!
//! Data region: concatenated payloads referenced by FileEntry.data_offset.
//! ```

use rpcs3_crypto::sha1_digest;

// ---------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------

/// PUP magic: "SCEUF\0\0\0" read as little-endian u64.
pub const PUP_MAGIC: u64 = 0x0000_0046_5545_4353;

pub const PUP_HEADER_SIZE: usize = 48;
pub const PUP_FILE_ENTRY_SIZE: usize = 32;
pub const PUP_HASH_ENTRY_SIZE: usize = 32;

// ---------------------------------------------------------------------
// Errors (mirror `enum class pup_error` in PUP.h:35-46)
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Stream unavailable.
    Stream,
    /// File is too short to contain the header.
    HeaderRead,
    /// Magic bytes do not match "SCEUF".
    HeaderMagic,
    /// `file_count` is nonsensical (zero or too large).
    HeaderFileCount,
    /// Computed header + data length exceeds file size.
    ExpectedSize,
    /// File entry table is malformed.
    FileEntries,
    /// One or more file-entry SHA-1 digests do not match their hash entry.
    HashMismatch { entry_id: u64 },
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Stream => f.write_str("PUP stream unavailable"),
            Error::HeaderRead => f.write_str("PUP header too short"),
            Error::HeaderMagic => f.write_str("PUP magic is not SCEUF"),
            Error::HeaderFileCount => f.write_str("PUP file_count is invalid"),
            Error::ExpectedSize => f.write_str("PUP declared size exceeds actual file"),
            Error::FileEntries => f.write_str("PUP file entry table malformed"),
            Error::HashMismatch { entry_id } => {
                write!(f, "PUP hash mismatch on entry {entry_id}")
            }
        }
    }
}

impl std::error::Error for Error {}

// ---------------------------------------------------------------------
// Header + entries
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct PupHeader {
    pub magic: u64,
    pub package_version: u64,
    pub image_version: u64,
    pub file_count: u64,
    pub header_length: u64,
    pub data_length: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct PupFileEntry {
    pub entry_id: u64,
    pub data_offset: u64,
    pub data_length: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct PupHashEntry {
    pub entry_id: u64,
    pub hash: [u8; 20],
}

// ---------------------------------------------------------------------
// Byte readers
// ---------------------------------------------------------------------

fn read_u64_le(buf: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(buf[at..at + 8].try_into().unwrap())
}

fn read_u64_be(buf: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(buf[at..at + 8].try_into().unwrap())
}

// ---------------------------------------------------------------------
// Parsed object
// ---------------------------------------------------------------------

/// A fully-parsed PUP file, with indexed access to individual payloads.
pub struct PupObject<'a> {
    bytes: &'a [u8],
    header: PupHeader,
    files: Vec<PupFileEntry>,
    hashes: Vec<PupHashEntry>,
}

impl<'a> core::fmt::Debug for PupObject<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PupObject")
            .field("bytes_len", &self.bytes.len())
            .field("header", &self.header)
            .field("files", &self.files.len())
            .field("hashes", &self.hashes.len())
            .finish()
    }
}

impl<'a> PupObject<'a> {
    /// Parse PUP bytes. Does NOT validate SHA-1 hashes. Call
    /// [`Self::validate_hashes`] when you want that (it's expensive).
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.is_empty() {
            return Err(Error::Stream);
        }
        if bytes.len() < PUP_HEADER_SIZE {
            return Err(Error::HeaderRead);
        }

        let header = PupHeader {
            magic: read_u64_le(bytes, 0),
            package_version: read_u64_be(bytes, 8),
            image_version: read_u64_be(bytes, 16),
            file_count: read_u64_be(bytes, 24),
            header_length: read_u64_be(bytes, 32),
            data_length: read_u64_be(bytes, 40),
        };

        if header.magic != PUP_MAGIC {
            return Err(Error::HeaderMagic);
        }

        if header.file_count == 0 {
            return Err(Error::HeaderFileCount);
        }

        let tables_size = PUP_FILE_ENTRY_SIZE as u64 * header.file_count
            + PUP_HASH_ENTRY_SIZE as u64 * header.file_count;
        if PUP_HEADER_SIZE as u64 + tables_size > header.header_length {
            return Err(Error::HeaderFileCount);
        }

        let total_needed = header.header_length.saturating_add(header.data_length);
        if total_needed > bytes.len() as u64 {
            return Err(Error::ExpectedSize);
        }

        let mut files = Vec::with_capacity(header.file_count as usize);
        let mut hashes = Vec::with_capacity(header.file_count as usize);

        for i in 0..header.file_count as usize {
            let base = PUP_HEADER_SIZE + i * PUP_FILE_ENTRY_SIZE;
            if base + PUP_FILE_ENTRY_SIZE > bytes.len() {
                return Err(Error::FileEntries);
            }
            files.push(PupFileEntry {
                entry_id: read_u64_be(bytes, base),
                data_offset: read_u64_be(bytes, base + 8),
                data_length: read_u64_be(bytes, base + 16),
            });
        }

        let hash_table_base = PUP_HEADER_SIZE + PUP_FILE_ENTRY_SIZE * header.file_count as usize;
        for i in 0..header.file_count as usize {
            let base = hash_table_base + i * PUP_HASH_ENTRY_SIZE;
            if base + PUP_HASH_ENTRY_SIZE > bytes.len() {
                return Err(Error::FileEntries);
            }
            let mut hash = [0u8; 20];
            hash.copy_from_slice(&bytes[base + 8..base + 28]);
            hashes.push(PupHashEntry {
                entry_id: read_u64_be(bytes, base),
                hash,
            });
        }

        // Sanity: each file's data region must fit in `bytes`.
        for f in &files {
            let end = f.data_offset.saturating_add(f.data_length);
            if end > bytes.len() as u64 {
                return Err(Error::FileEntries);
            }
        }

        Ok(Self { bytes, header, files, hashes })
    }

    pub fn header(&self) -> &PupHeader {
        &self.header
    }

    pub fn files(&self) -> &[PupFileEntry] {
        &self.files
    }

    pub fn hashes(&self) -> &[PupHashEntry] {
        &self.hashes
    }

    /// Retrieve the raw bytes of a file by `entry_id`.
    /// Returns `None` if no file entry has that id.
    pub fn get_file(&self, entry_id: u64) -> Option<&[u8]> {
        let entry = self.files.iter().find(|e| e.entry_id == entry_id)?;
        let start = entry.data_offset as usize;
        let end = start + entry.data_length as usize;
        self.bytes.get(start..end)
    }

    /// Validate all entry hashes (SHA-1 of file bytes == hash entry).
    /// Matches `pup_object::validate_hashes()` (PUP.cpp).
    pub fn validate_hashes(&self) -> Result<(), Error> {
        for hash_entry in &self.hashes {
            let data = self
                .get_file(hash_entry.entry_id)
                .ok_or(Error::HashMismatch { entry_id: hash_entry.entry_id })?;
            let digest = sha1_digest(data);
            if digest != hash_entry.hash {
                return Err(Error::HashMismatch { entry_id: hash_entry.entry_id });
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal PUP with `n` files, each containing `payload[i]`.
    /// Returns bytes + expected `entry_id` list.
    fn build_pup(payloads: &[&[u8]]) -> Vec<u8> {
        let n = payloads.len() as u64;
        let header_len = PUP_HEADER_SIZE as u64
            + PUP_FILE_ENTRY_SIZE as u64 * n
            + PUP_HASH_ENTRY_SIZE as u64 * n;

        let mut data_len: u64 = 0;
        let mut data_offsets = Vec::with_capacity(payloads.len());
        for p in payloads {
            data_offsets.push(header_len + data_len);
            data_len += p.len() as u64;
        }

        let total = (header_len + data_len) as usize;
        let mut bytes = vec![0u8; total];

        // Header
        bytes[0..8].copy_from_slice(&PUP_MAGIC.to_le_bytes());
        bytes[8..16].copy_from_slice(&0x0001_0000u64.to_be_bytes()); // package_version
        bytes[16..24].copy_from_slice(&0x0001_0000u64.to_be_bytes()); // image_version
        bytes[24..32].copy_from_slice(&n.to_be_bytes()); // file_count
        bytes[32..40].copy_from_slice(&header_len.to_be_bytes());
        bytes[40..48].copy_from_slice(&data_len.to_be_bytes());

        // File table
        for (i, p) in payloads.iter().enumerate() {
            let base = PUP_HEADER_SIZE + i * PUP_FILE_ENTRY_SIZE;
            bytes[base..base + 8].copy_from_slice(&(100u64 + i as u64).to_be_bytes()); // entry_id
            bytes[base + 8..base + 16].copy_from_slice(&data_offsets[i].to_be_bytes());
            bytes[base + 16..base + 24].copy_from_slice(&(p.len() as u64).to_be_bytes());
        }

        // Hash table
        let hash_base = PUP_HEADER_SIZE + PUP_FILE_ENTRY_SIZE * payloads.len();
        for (i, p) in payloads.iter().enumerate() {
            let base = hash_base + i * PUP_HASH_ENTRY_SIZE;
            bytes[base..base + 8].copy_from_slice(&(100u64 + i as u64).to_be_bytes());
            bytes[base + 8..base + 28].copy_from_slice(&sha1_digest(p));
        }

        // Data payloads
        for (i, p) in payloads.iter().enumerate() {
            let start = data_offsets[i] as usize;
            bytes[start..start + p.len()].copy_from_slice(p);
        }

        bytes
    }

    #[test]
    fn parse_empty_is_stream_error() {
        assert_eq!(PupObject::parse(&[]).unwrap_err(), Error::Stream);
    }

    #[test]
    fn parse_too_short_is_header_read() {
        assert_eq!(PupObject::parse(&[0u8; 20]).unwrap_err(), Error::HeaderRead);
    }

    #[test]
    fn parse_wrong_magic_is_header_magic() {
        let mut bytes = vec![0u8; 64];
        bytes[0..8].copy_from_slice(b"NOTSCEUF");
        assert_eq!(PupObject::parse(&bytes).unwrap_err(), Error::HeaderMagic);
    }

    #[test]
    fn parse_zero_file_count_is_rejected() {
        let mut bytes = build_pup(&[b"payload"]);
        // Stomp file_count to 0
        bytes[24..32].copy_from_slice(&0u64.to_be_bytes());
        assert_eq!(PupObject::parse(&bytes).unwrap_err(), Error::HeaderFileCount);
    }

    #[test]
    fn parse_single_file_roundtrip() {
        let payload: &[u8] = b"hello firmware world";
        let bytes = build_pup(&[payload]);
        let pup = PupObject::parse(&bytes).unwrap();
        assert_eq!(pup.header().magic, PUP_MAGIC);
        assert_eq!(pup.header().file_count, 1);
        assert_eq!(pup.files().len(), 1);
        assert_eq!(pup.files()[0].entry_id, 100);
        assert_eq!(pup.get_file(100), Some(payload));
        assert_eq!(pup.get_file(999), None);
    }

    #[test]
    fn parse_multi_file_roundtrip() {
        let payloads: &[&[u8]] = &[b"fileA", b"fileBB", b"fileCCC"];
        let bytes = build_pup(payloads);
        let pup = PupObject::parse(&bytes).unwrap();
        assert_eq!(pup.files().len(), 3);
        assert_eq!(pup.get_file(100), Some(payloads[0]));
        assert_eq!(pup.get_file(101), Some(payloads[1]));
        assert_eq!(pup.get_file(102), Some(payloads[2]));
    }

    #[test]
    fn validate_hashes_passes_for_correct_hashes() {
        let payloads: &[&[u8]] = &[b"A", b"BB", b"CCC"];
        let bytes = build_pup(payloads);
        let pup = PupObject::parse(&bytes).unwrap();
        pup.validate_hashes().unwrap();
    }

    #[test]
    fn validate_hashes_detects_tampered_payload() {
        let payloads: &[&[u8]] = &[b"original"];
        let mut bytes = build_pup(payloads);
        // Tamper the payload byte at its actual offset.
        let payload_offset = PUP_HEADER_SIZE + PUP_FILE_ENTRY_SIZE + PUP_HASH_ENTRY_SIZE;
        bytes[payload_offset] ^= 0xFF;
        let pup = PupObject::parse(&bytes).unwrap();
        let err = pup.validate_hashes().unwrap_err();
        assert!(matches!(err, Error::HashMismatch { entry_id: 100 }));
    }

    #[test]
    fn magic_constant_decodes_to_sceuf() {
        // "SCEUF\0\0\0" as u64 little-endian must be PUP_MAGIC.
        let expected = u64::from_le_bytes(*b"SCEUF\0\0\0");
        assert_eq!(expected, PUP_MAGIC);
    }

    #[test]
    fn parse_detects_data_longer_than_file() {
        let payload: &[u8] = b"hi";
        let mut bytes = build_pup(&[payload]);
        // Lie about data_length: claim much more than file has.
        bytes[40..48].copy_from_slice(&0xFFFF_FFFFu64.to_be_bytes());
        assert_eq!(PupObject::parse(&bytes).unwrap_err(), Error::ExpectedSize);
    }
}
