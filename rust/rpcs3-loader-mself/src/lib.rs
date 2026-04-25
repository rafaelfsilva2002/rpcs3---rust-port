//! `rpcs3-loader-mself` — MSELF (multi-SELF) container parser.
//!
//! Mirrors `rpcs3/Loader/mself.hpp`. MSELF is a simple container used in
//! PS3 firmware to bundle multiple SELFs in a single file.
//!
//! ## Format (all BE except noted)
//!
//! ```text
//! Header (64 bytes):
//!   magic        u32 BE  = "MSF\0" (0x4D534600)
//!   ver          u32 BE  = 1
//!   size         u64 BE  = total file size (self-referential sanity check)
//!   count        u32 BE  = number of records
//!   header_size  u32 BE
//!   reserved     u8[40]
//!
//! Record[count] (64 bytes each):
//!   name      char[32]  — NUL-padded ASCII filename
//!   off       u64 BE    — offset into the MSELF file
//!   size      u64 BE    — payload size
//!   reserved  u8[16]
//!
//! Payloads follow the record table, each at its record's `off`.
//! ```

// ---------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------

/// "MSF\0" in big-endian.
pub const MSELF_MAGIC: u32 = 0x4D53_4600;

pub const MSELF_HEADER_SIZE: usize = 64;
pub const MSELF_RECORD_SIZE: usize = 64;

// ---------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct MselfHeader {
    pub magic: u32,
    pub ver: u32,
    pub size: u64,
    pub count: u32,
    pub header_size: u32,
}

#[derive(Debug, Clone)]
pub struct MselfRecord {
    pub name: String,
    pub off: u64,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    TooShort,
    BadMagic,
    BadVersion(u32),
    /// Header's declared total size doesn't match input.
    SizeMismatch { expected: u64, actual: u64 },
    /// Record's offset+size reaches beyond the input buffer.
    RecordOutOfBounds { index: u32, off: u64, size: u64 },
    /// Record's filename is not valid ASCII.
    BadRecordName,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::TooShort => f.write_str("MSELF input too short"),
            Error::BadMagic => f.write_str("MSELF magic is not MSF\\0"),
            Error::BadVersion(v) => write!(f, "MSELF version {v} unsupported (expected 1)"),
            Error::SizeMismatch { expected, actual } => {
                write!(f, "MSELF declared size {expected} != actual {actual}")
            }
            Error::RecordOutOfBounds { index, off, size } => {
                write!(f, "MSELF record {index} payload at off={off:#x} size={size:#x} exceeds file")
            }
            Error::BadRecordName => f.write_str("MSELF record name contains non-ASCII bytes"),
        }
    }
}

impl std::error::Error for Error {}

pub struct MselfObject<'a> {
    bytes: &'a [u8],
    header: MselfHeader,
    records: Vec<MselfRecord>,
}

impl<'a> core::fmt::Debug for MselfObject<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MselfObject")
            .field("bytes_len", &self.bytes.len())
            .field("header", &self.header)
            .field("records", &self.records.len())
            .finish()
    }
}

// ---------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------

fn read_u32_be(buf: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(buf[at..at + 4].try_into().unwrap())
}

fn read_u64_be(buf: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(buf[at..at + 8].try_into().unwrap())
}

impl<'a> MselfObject<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() < MSELF_HEADER_SIZE {
            return Err(Error::TooShort);
        }

        let magic = read_u32_be(bytes, 0);
        if magic != MSELF_MAGIC {
            return Err(Error::BadMagic);
        }

        let ver = read_u32_be(bytes, 4);
        if ver != 1 {
            return Err(Error::BadVersion(ver));
        }

        let size = read_u64_be(bytes, 8);
        let count = read_u32_be(bytes, 16);
        let header_size = read_u32_be(bytes, 20);

        let actual = bytes.len() as u64;
        if size != actual {
            return Err(Error::SizeMismatch { expected: size, actual });
        }

        // Sanity: the record table must fit between header and content.
        let records_needed = count as u64 * MSELF_RECORD_SIZE as u64;
        if MSELF_HEADER_SIZE as u64 + records_needed > actual {
            return Err(Error::TooShort);
        }

        let mut records = Vec::with_capacity(count as usize);
        for i in 0..count as usize {
            let base = MSELF_HEADER_SIZE + i * MSELF_RECORD_SIZE;

            let name_bytes = &bytes[base..base + 32];
            let nul = name_bytes.iter().position(|&b| b == 0).unwrap_or(32);
            let raw_name = &name_bytes[..nul];
            if !raw_name.is_ascii() {
                return Err(Error::BadRecordName);
            }
            let name = std::str::from_utf8(raw_name).unwrap().to_owned();

            let off = read_u64_be(bytes, base + 32);
            let size_r = read_u64_be(bytes, base + 40);

            let end = off.saturating_add(size_r);
            if end > actual {
                return Err(Error::RecordOutOfBounds {
                    index: i as u32,
                    off,
                    size: size_r,
                });
            }

            records.push(MselfRecord { name, off, size: size_r });
        }

        Ok(Self {
            bytes,
            header: MselfHeader { magic, ver, size, count, header_size },
            records,
        })
    }

    pub fn header(&self) -> &MselfHeader {
        &self.header
    }

    pub fn records(&self) -> &[MselfRecord] {
        &self.records
    }

    /// Retrieve payload bytes by record index.
    pub fn get_by_index(&self, index: usize) -> Option<&[u8]> {
        let r = self.records.get(index)?;
        self.bytes.get(r.off as usize..(r.off + r.size) as usize)
    }

    /// Retrieve payload bytes by record name.
    pub fn get_by_name(&self, name: &str) -> Option<&[u8]> {
        let idx = self.records.iter().position(|r| r.name == name)?;
        self.get_by_index(idx)
    }
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal MSELF with N records each containing `payloads[i]`.
    fn build_mself(records: &[(&str, &[u8])]) -> Vec<u8> {
        let count = records.len();
        let header_size = MSELF_HEADER_SIZE as u64;
        let record_table_size = (count * MSELF_RECORD_SIZE) as u64;

        let mut payload_off = header_size + record_table_size;
        let mut offsets = Vec::new();
        let mut total_payload: u64 = 0;
        for (_, p) in records {
            offsets.push(payload_off + total_payload);
            total_payload += p.len() as u64;
        }
        let total_size = header_size + record_table_size + total_payload;

        let mut bytes = vec![0u8; total_size as usize];

        // Header
        bytes[0..4].copy_from_slice(&MSELF_MAGIC.to_be_bytes());
        bytes[4..8].copy_from_slice(&1u32.to_be_bytes());
        bytes[8..16].copy_from_slice(&total_size.to_be_bytes());
        bytes[16..20].copy_from_slice(&(count as u32).to_be_bytes());
        bytes[20..24].copy_from_slice(&(header_size as u32).to_be_bytes());

        // Records
        for (i, (name, payload)) in records.iter().enumerate() {
            let base = MSELF_HEADER_SIZE + i * MSELF_RECORD_SIZE;
            let nbytes = name.as_bytes();
            bytes[base..base + nbytes.len()].copy_from_slice(nbytes);
            bytes[base + 32..base + 40].copy_from_slice(&offsets[i].to_be_bytes());
            bytes[base + 40..base + 48].copy_from_slice(&(payload.len() as u64).to_be_bytes());
            // Payload
            let start = offsets[i] as usize;
            bytes[start..start + payload.len()].copy_from_slice(payload);
        }

        bytes
    }

    #[test]
    fn parse_rejects_short_input() {
        assert_eq!(MselfObject::parse(&[0u8; 32]).unwrap_err(), Error::TooShort);
    }

    #[test]
    fn parse_rejects_bad_magic() {
        let mut bytes = build_mself(&[("file.bin", b"data")]);
        bytes[0..4].copy_from_slice(b"XXXX");
        assert_eq!(MselfObject::parse(&bytes).unwrap_err(), Error::BadMagic);
    }

    #[test]
    fn parse_rejects_bad_version() {
        let mut bytes = build_mself(&[("file.bin", b"data")]);
        bytes[4..8].copy_from_slice(&99u32.to_be_bytes());
        assert_eq!(MselfObject::parse(&bytes).unwrap_err(), Error::BadVersion(99));
    }

    #[test]
    fn parse_rejects_size_mismatch() {
        let mut bytes = build_mself(&[("file.bin", b"data")]);
        bytes[8..16].copy_from_slice(&0xDEAD_BEEFu64.to_be_bytes());
        assert!(matches!(MselfObject::parse(&bytes), Err(Error::SizeMismatch { .. })));
    }

    #[test]
    fn parse_single_record_roundtrip() {
        let bytes = build_mself(&[("EBOOT.BIN", b"payload bytes")]);
        let m = MselfObject::parse(&bytes).unwrap();
        assert_eq!(m.header().magic, MSELF_MAGIC);
        assert_eq!(m.header().count, 1);
        assert_eq!(m.records().len(), 1);
        assert_eq!(m.records()[0].name, "EBOOT.BIN");
        assert_eq!(m.get_by_index(0), Some(b"payload bytes".as_ref()));
        assert_eq!(m.get_by_name("EBOOT.BIN"), Some(b"payload bytes".as_ref()));
        assert_eq!(m.get_by_name("not-present"), None);
    }

    #[test]
    fn parse_multiple_records_roundtrip() {
        let records: &[(&str, &[u8])] = &[
            ("kernel.self", b"A"),
            ("drv_net.self", b"BB"),
            ("storage.self", b"CCC"),
        ];
        let bytes = build_mself(records);
        let m = MselfObject::parse(&bytes).unwrap();
        assert_eq!(m.records().len(), 3);
        assert_eq!(m.get_by_name("kernel.self"), Some(b"A".as_ref()));
        assert_eq!(m.get_by_name("drv_net.self"), Some(b"BB".as_ref()));
        assert_eq!(m.get_by_name("storage.self"), Some(b"CCC".as_ref()));
    }

    #[test]
    fn parse_rejects_out_of_bounds_record() {
        let mut bytes = build_mself(&[("x", b"y")]);
        // Tamper record offset to exceed file size
        let record_base = MSELF_HEADER_SIZE;
        bytes[record_base + 32..record_base + 40].copy_from_slice(&0xFFFF_FFFFu64.to_be_bytes());
        assert!(matches!(
            MselfObject::parse(&bytes),
            Err(Error::RecordOutOfBounds { .. })
        ));
    }

    #[test]
    fn parse_rejects_non_ascii_record_name() {
        let mut bytes = build_mself(&[("ok", b"data")]);
        // Corrupt the name bytes
        bytes[MSELF_HEADER_SIZE] = 0xFF;
        bytes[MSELF_HEADER_SIZE + 1] = 0xFE;
        assert_eq!(MselfObject::parse(&bytes).unwrap_err(), Error::BadRecordName);
    }

    #[test]
    fn magic_constant_decodes_correctly() {
        let expected = u32::from_be_bytes(*b"MSF\0");
        assert_eq!(expected, MSELF_MAGIC);
    }
}
