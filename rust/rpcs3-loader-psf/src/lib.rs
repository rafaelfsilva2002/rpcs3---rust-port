//! `rpcs3-loader-psf` — PARAM.SFO parser/emitter.
//!
//! Behavioral parity with `rpcs3/Loader/PSF.{h,cpp}`.
//!
//! ## File format
//!
//! ```text
//! Header (20 bytes, little-endian):
//!   magic          u32  = 0x46535000 ("\0PSF")
//!   version        u32  = 0x00000101
//!   off_key_table  u32
//!   off_data_table u32
//!   entries_num    u32
//!
//! Def table entries × entries_num (16 bytes each):
//!   key_off    u16  — offset into key table
//!   param_fmt  u16  — 0x0004 array | 0x0204 string | 0x0404 integer
//!   param_len  u32  — actual data length
//!   param_max  u32  — reserved slot size
//!   data_off   u32  — offset into data table
//!
//! Key table: null-terminated UTF-8 strings, padded to 4 bytes.
//! Data table: values laid out at data_off, size param_max.
//! ```
//!
//! ## Load semantics (mirror of `psf::load`, PSF.cpp:182)
//!
//! * Missing/unreadable stream → `Error::Stream`.
//! * Wrong magic or version → `Error::NotPsf`.
//! * Any structural inconsistency → `Error::Corrupt`.
//! * On error: `sfo` is cleared, `err` set; caller gets empty registry.
//! * Unknown format per entry: entry is ignored (not fatal).
//! * Missing/invalid `CATEGORY` field → `Error::Corrupt` (matches PSF.cpp:273-280).
//!
//! ## Save semantics (mirror of `psf::save_object`, PSF.cpp:291)
//!
//! * Keys are emitted in lexicographic order (BTreeMap matches std::map).
//! * Strings are NUL-terminated inside their `param_max` slot.
//! * Key table end padded to 4-byte alignment before data table.

use std::collections::BTreeMap;

// ---------------------------------------------------------------------
// Constants from rpcs3/Loader/PSF.{h,cpp}
// ---------------------------------------------------------------------

const MAGIC: u32 = 0x46535000; // "\0PSF" little-endian
const VERSION: u32 = 0x0000_0101;

const FORMAT_ARRAY: u16 = 0x0004;
const FORMAT_STRING: u16 = 0x0204;
const FORMAT_INTEGER: u16 = 0x0404;

const HEADER_SIZE: usize = 20;
const DEF_TABLE_SIZE: usize = 16;

/// Valid CATEGORY values, mirroring PSF.cpp:274.
const VALID_CATEGORIES: &[&str] = &[
    "GD", "DG", "HG", "AM", "AP", "AS", "AT", "AV", "BV", "WT", "HM", "CB", "SF", "2P", "2G",
    "1P", "PP", "MN", "PE", "2D", "SD", "MS",
];

// ---------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------

/// One PSF registry entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// Unsigned 32-bit integer (format 0x0404).
    Integer(u32),
    /// NUL-terminated string (format 0x0204). `max_size` reserves slot space
    /// including the trailing NUL.
    String { value: String, max_size: u32 },
    /// Fixed-length byte array (format 0x0004), not NUL-terminated.
    Array { value: Vec<u8>, max_size: u32 },
}

impl Entry {
    /// Size written to disk — `u32::size()` from C++.
    fn size_on_disk(&self) -> u32 {
        match self {
            Entry::Integer(_) => 4,
            Entry::String { value, max_size } => {
                let needed = u32::try_from(value.len()).unwrap_or(u32::MAX).saturating_add(1);
                needed.min(*max_size)
            }
            Entry::Array { value, max_size } => {
                let needed = u32::try_from(value.len()).unwrap_or(u32::MAX);
                needed.min(*max_size)
            }
        }
    }

    /// `max_size` used in def_table (matches `entry::max(with_nts=true)`).
    fn max_on_disk(&self) -> u32 {
        match self {
            Entry::Integer(_) => 4,
            Entry::String { max_size, .. } | Entry::Array { max_size, .. } => *max_size,
        }
    }

    fn format_code(&self) -> u16 {
        match self {
            Entry::Integer(_) => FORMAT_INTEGER,
            Entry::String { .. } => FORMAT_STRING,
            Entry::Array { .. } => FORMAT_ARRAY,
        }
    }
}

/// Errors observable to the caller, matching `psf::error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// Source stream is missing/unreadable. We return this if input bytes
    /// are empty (closest analogue to `!static_cast<bool>(stream)`).
    Stream,
    /// File exists but magic or version is wrong.
    NotPsf,
    /// Structural inconsistency or invalid CATEGORY.
    Corrupt,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Stream => f.write_str("PSF stream unavailable"),
            Error::NotPsf => f.write_str("file is not of PSF format"),
            Error::Corrupt => f.write_str("PSF is truncated or corrupted"),
        }
    }
}

impl std::error::Error for Error {}

/// Ordered map by key (matches `std::map<std::string, entry, std::less<>>`).
pub type Registry = BTreeMap<String, Entry>;

/// Return type of [`load`], matching `psf::load_result_t`.
#[derive(Debug, Default)]
pub struct LoadResult {
    pub sfo: Registry,
    pub err: Option<Error>,
}

impl LoadResult {
    fn fail(err: Error) -> Self {
        Self { sfo: Registry::new(), err: Some(err) }
    }
}

// ---------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------

fn read_u16_le(buf: &[u8], at: usize) -> Option<u16> {
    buf.get(at..at + 2).and_then(|s| s.try_into().ok()).map(u16::from_le_bytes)
}

fn read_u32_le(buf: &[u8], at: usize) -> Option<u32> {
    buf.get(at..at + 4).and_then(|s| s.try_into().ok()).map(u32::from_le_bytes)
}

#[derive(Debug)]
struct DefTableEntry {
    key_off: u16,
    param_fmt: u16,
    param_len: u32,
    param_max: u32,
    data_off: u32,
}

/// Parse PARAM.SFO bytes. Mirrors `psf::load` (PSF.cpp:182).
#[must_use]
pub fn load(bytes: &[u8]) -> LoadResult {
    if bytes.is_empty() {
        return LoadResult::fail(Error::Stream);
    }

    // Header
    if bytes.len() < HEADER_SIZE {
        return LoadResult::fail(Error::NotPsf);
    }

    let magic = read_u32_le(bytes, 0).unwrap();
    let version = read_u32_le(bytes, 4).unwrap();
    let off_key_table = read_u32_le(bytes, 8).unwrap();
    let off_data_table = read_u32_le(bytes, 12).unwrap();
    let entries_num = read_u32_le(bytes, 16).unwrap();

    if magic != MAGIC || version != VERSION {
        return LoadResult::fail(Error::NotPsf);
    }

    // Offsets sanity — mirror C++ checks at PSF.cpp:207-209
    if u64::from(off_key_table) < HEADER_SIZE as u64 {
        return LoadResult::fail(Error::Corrupt);
    }
    if off_key_table > off_data_table {
        return LoadResult::fail(Error::Corrupt);
    }
    if off_data_table as usize > bytes.len() {
        return LoadResult::fail(Error::Corrupt);
    }

    // Read def table
    let mut defs = Vec::with_capacity(entries_num as usize);
    let defs_start = HEADER_SIZE;
    let defs_end = defs_start + DEF_TABLE_SIZE * entries_num as usize;
    if defs_end > off_key_table as usize {
        return LoadResult::fail(Error::Corrupt);
    }
    for i in 0..entries_num as usize {
        let base = defs_start + i * DEF_TABLE_SIZE;
        let Some(key_off) = read_u16_le(bytes, base) else {
            return LoadResult::fail(Error::Corrupt);
        };
        let Some(param_fmt) = read_u16_le(bytes, base + 2) else {
            return LoadResult::fail(Error::Corrupt);
        };
        let Some(param_len) = read_u32_le(bytes, base + 4) else {
            return LoadResult::fail(Error::Corrupt);
        };
        let Some(param_max) = read_u32_le(bytes, base + 8) else {
            return LoadResult::fail(Error::Corrupt);
        };
        let Some(data_off) = read_u32_le(bytes, base + 12) else {
            return LoadResult::fail(Error::Corrupt);
        };
        defs.push(DefTableEntry { key_off, param_fmt, param_len, param_max, data_off });
    }

    // Key table slice
    let key_slice = match bytes.get(off_key_table as usize..off_data_table as usize) {
        Some(s) => s,
        None => return LoadResult::fail(Error::Corrupt),
    };
    let key_table_len = key_slice.len() as u32;

    let data_slice = match bytes.get(off_data_table as usize..) {
        Some(s) => s,
        None => return LoadResult::fail(Error::Corrupt),
    };

    let mut sfo = Registry::new();

    for def in &defs {
        // PSF.cpp:223: key_off must fit in key table
        if u32::from(def.key_off) >= key_table_len {
            return LoadResult::fail(Error::Corrupt);
        }

        // Read NUL-terminated key
        let key_bytes = &key_slice[def.key_off as usize..];
        let nul = match key_bytes.iter().position(|&b| b == 0) {
            Some(n) => n,
            None => return LoadResult::fail(Error::Corrupt),
        };
        let key = match std::str::from_utf8(&key_bytes[..nul]) {
            Ok(s) => s.to_owned(),
            Err(_) => return LoadResult::fail(Error::Corrupt),
        };

        // PSF.cpp:229: no duplicates
        if sfo.contains_key(&key) {
            return LoadResult::fail(Error::Corrupt);
        }

        // PSF.cpp:230: param_len <= param_max
        if def.param_len > def.param_max {
            return LoadResult::fail(Error::Corrupt);
        }

        // PSF.cpp:231-232: data_off + param_max bounded by data section size
        let available = data_slice.len() as u64;
        if u64::from(def.data_off) >= available {
            return LoadResult::fail(Error::Corrupt);
        }
        if u64::from(def.data_off) + u64::from(def.param_max) > available {
            return LoadResult::fail(Error::Corrupt);
        }

        let start = def.data_off as usize;

        match def.param_fmt {
            FORMAT_INTEGER if def.param_max == 4 && def.param_len == 4 => {
                let Some(value) = read_u32_le(data_slice, start) else {
                    return LoadResult::fail(Error::Corrupt);
                };
                sfo.insert(key, Entry::Integer(value));
            }
            FORMAT_STRING => {
                let end = start + def.param_len as usize;
                let raw = &data_slice[start..end];
                // Truncate at first NUL (PSF.cpp:253-258)
                let trimmed_len = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
                let s = match std::str::from_utf8(&raw[..trimmed_len]) {
                    Ok(s) => s.to_owned(),
                    Err(_) => return LoadResult::fail(Error::Corrupt),
                };
                sfo.insert(
                    key,
                    Entry::String { value: s, max_size: def.param_max },
                );
            }
            FORMAT_ARRAY => {
                let end = start + def.param_len as usize;
                let raw = data_slice[start..end].to_vec();
                sfo.insert(
                    key,
                    Entry::Array { value: raw, max_size: def.param_max },
                );
            }
            _ => {
                // Unknown format: ignore entry, keep going (PSF.cpp:267-270)
                continue;
            }
        }
    }

    // PSF.cpp:273-280: validate CATEGORY
    let cat = get_string(&sfo, "CATEGORY", "");
    if !VALID_CATEGORIES.contains(&cat) {
        return LoadResult::fail(Error::Corrupt);
    }

    LoadResult { sfo, err: None }
}

// ---------------------------------------------------------------------
// Serialization (mirror of psf::save_object, PSF.cpp:291)
// ---------------------------------------------------------------------

/// Serialize a registry to PSF bytes. Deterministic: keys ordered
/// lexicographically (matches `std::map`).
#[must_use]
pub fn save(sfo: &Registry) -> Vec<u8> {
    // First pass: compute offsets
    let entries: Vec<(&String, &Entry)> = sfo.iter().collect();

    let mut key_offsets = Vec::with_capacity(entries.len());
    let mut data_offsets = Vec::with_capacity(entries.len());

    let mut key_cursor: usize = 0;
    let mut data_cursor: usize = 0;

    for (key, entry) in &entries {
        key_offsets.push(key_cursor);
        data_offsets.push(data_cursor);
        key_cursor += key.len() + 1; // NUL terminator
        data_cursor += entry.max_on_disk() as usize;
    }

    // Align end of key table to 4 bytes (PSF.cpp:317)
    let key_table_aligned = (key_cursor + 3) & !3;

    let off_key_table = (HEADER_SIZE + DEF_TABLE_SIZE * entries.len()) as u32;
    let off_data_table = off_key_table + key_table_aligned as u32;
    let total_size = off_data_table as usize + data_cursor;

    let mut out = vec![0u8; total_size];

    // Header
    out[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    out[4..8].copy_from_slice(&VERSION.to_le_bytes());
    out[8..12].copy_from_slice(&off_key_table.to_le_bytes());
    out[12..16].copy_from_slice(&off_data_table.to_le_bytes());
    out[16..20].copy_from_slice(&(entries.len() as u32).to_le_bytes());

    // Def table
    for (i, (_key, entry)) in entries.iter().enumerate() {
        let base = HEADER_SIZE + i * DEF_TABLE_SIZE;
        let key_off = key_offsets[i] as u16;
        let data_off = data_offsets[i] as u32;

        out[base..base + 2].copy_from_slice(&key_off.to_le_bytes());
        out[base + 2..base + 4].copy_from_slice(&entry.format_code().to_le_bytes());
        out[base + 4..base + 8].copy_from_slice(&entry.size_on_disk().to_le_bytes());
        out[base + 8..base + 12].copy_from_slice(&entry.max_on_disk().to_le_bytes());
        out[base + 12..base + 16].copy_from_slice(&data_off.to_le_bytes());
    }

    // Key table
    let mut kcursor = off_key_table as usize;
    for (key, _) in &entries {
        let bytes = key.as_bytes();
        out[kcursor..kcursor + bytes.len()].copy_from_slice(bytes);
        kcursor += bytes.len();
        out[kcursor] = 0; // NUL
        kcursor += 1;
    }
    // Remaining key table slots already zero-filled (alignment padding).

    // Data table
    let data_base = off_data_table as usize;
    for (i, (_key, entry)) in entries.iter().enumerate() {
        let start = data_base + data_offsets[i];
        let max = entry.max_on_disk() as usize;
        match entry {
            Entry::Integer(v) => {
                out[start..start + 4].copy_from_slice(&v.to_le_bytes());
            }
            Entry::String { value, .. } => {
                let bytes = value.as_bytes();
                let write_len = bytes.len().min(max.saturating_sub(1));
                out[start..start + write_len].copy_from_slice(&bytes[..write_len]);
                // Remaining bytes already zero (NUL + padding).
            }
            Entry::Array { value, .. } => {
                let write_len = value.len().min(max);
                out[start..start + write_len].copy_from_slice(&value[..write_len]);
            }
        }
    }

    out
}

// ---------------------------------------------------------------------
// Accessors (mirror psf::get_string / get_integer)
// ---------------------------------------------------------------------

/// Get string/array value as `&str`, else default.
///
/// Mirrors `psf::get_string` (PSF.cpp:375): strings and arrays both
/// return their bytes interpreted as text; integer entries return
/// the default. For arrays that are not valid UTF-8, we return the
/// default — the caller can still read raw bytes from the registry.
#[must_use]
pub fn get_string<'a>(sfo: &'a Registry, key: &str, default: &'a str) -> &'a str {
    match sfo.get(key) {
        Some(Entry::String { value, .. }) => value.as_str(),
        Some(Entry::Array { value, .. }) => std::str::from_utf8(value).unwrap_or(default),
        _ => default,
    }
}

/// Get integer value, else default. String/array entries return default.
#[must_use]
pub fn get_integer(sfo: &Registry, key: &str, default: u32) -> u32 {
    match sfo.get(key) {
        Some(Entry::Integer(v)) => *v,
        _ => default,
    }
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal-but-valid PSF with one CATEGORY entry.
    fn build_minimal(cat: &str) -> Vec<u8> {
        let mut reg = Registry::new();
        reg.insert(
            "CATEGORY".to_owned(),
            Entry::String { value: cat.to_owned(), max_size: 4 },
        );
        save(&reg)
    }

    // -- Magic / header --------------------------------------------

    #[test]
    fn empty_input_is_stream_error() {
        let r = load(&[]);
        assert_eq!(r.err, Some(Error::Stream));
        assert!(r.sfo.is_empty());
    }

    #[test]
    fn too_short_is_not_psf() {
        let r = load(&[0, 1, 2, 3]);
        assert_eq!(r.err, Some(Error::NotPsf));
    }

    #[test]
    fn wrong_magic_is_not_psf() {
        let mut bytes = vec![0u8; HEADER_SIZE];
        bytes[0..4].copy_from_slice(b"XXXX");
        let r = load(&bytes);
        assert_eq!(r.err, Some(Error::NotPsf));
    }

    #[test]
    fn wrong_version_is_not_psf() {
        let mut bytes = vec![0u8; HEADER_SIZE];
        bytes[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        bytes[4..8].copy_from_slice(&0x999u32.to_le_bytes());
        let r = load(&bytes);
        assert_eq!(r.err, Some(Error::NotPsf));
    }

    // -- Round-trip ------------------------------------------------

    #[test]
    fn roundtrip_minimal_category() {
        let bytes = build_minimal("HG");
        let r = load(&bytes);
        assert!(r.err.is_none(), "load error: {:?}", r.err);
        assert_eq!(get_string(&r.sfo, "CATEGORY", ""), "HG");
    }

    #[test]
    fn roundtrip_integer_entry() {
        let mut reg = Registry::new();
        reg.insert("CATEGORY".to_owned(), Entry::String { value: "HG".to_owned(), max_size: 4 });
        reg.insert("PARENTAL_LEVEL".to_owned(), Entry::Integer(5));
        let bytes = save(&reg);
        let r = load(&bytes);
        assert!(r.err.is_none());
        assert_eq!(get_integer(&r.sfo, "PARENTAL_LEVEL", 0), 5);
        assert_eq!(get_integer(&r.sfo, "NONEXISTENT", 42), 42);
    }

    #[test]
    fn roundtrip_multiple_entries_sorted() {
        let mut reg = Registry::new();
        reg.insert("CATEGORY".to_owned(), Entry::String { value: "HG".to_owned(), max_size: 4 });
        reg.insert("TITLE".to_owned(), Entry::String { value: "Test Game".to_owned(), max_size: 128 });
        reg.insert("TITLE_ID".to_owned(), Entry::String { value: "BLES01234".to_owned(), max_size: 16 });
        reg.insert("APP_VER".to_owned(), Entry::String { value: "01.00".to_owned(), max_size: 8 });
        reg.insert("PARENTAL_LEVEL".to_owned(), Entry::Integer(2));
        let bytes = save(&reg);
        let r = load(&bytes);
        assert!(r.err.is_none());
        assert_eq!(r.sfo.len(), 5);
        assert_eq!(get_string(&r.sfo, "TITLE_ID", ""), "BLES01234");
        assert_eq!(get_string(&r.sfo, "TITLE", ""), "Test Game");
        assert_eq!(get_string(&r.sfo, "APP_VER", ""), "01.00");
        assert_eq!(get_integer(&r.sfo, "PARENTAL_LEVEL", 0), 2);
    }

    #[test]
    fn roundtrip_array_entry() {
        let mut reg = Registry::new();
        reg.insert("CATEGORY".to_owned(), Entry::String { value: "HG".to_owned(), max_size: 4 });
        let arr = vec![0xDEu8, 0xAD, 0xBE, 0xEF, 0x00, 0x00, 0x00, 0x00];
        reg.insert("ATTRIBUTE".to_owned(), Entry::Array { value: arr.clone(), max_size: 8 });
        let bytes = save(&reg);
        let r = load(&bytes);
        assert!(r.err.is_none());
        match r.sfo.get("ATTRIBUTE") {
            Some(Entry::Array { value, max_size }) => {
                assert_eq!(value, &arr);
                assert_eq!(*max_size, 8);
            }
            other => panic!("expected array, got {other:?}"),
        }
    }

    // -- CATEGORY validation ---------------------------------------

    #[test]
    fn invalid_category_is_corrupt() {
        let bytes = build_minimal("XX"); // "XX" not in VALID_CATEGORIES
        let r = load(&bytes);
        assert_eq!(r.err, Some(Error::Corrupt));
        assert!(r.sfo.is_empty());
    }

    #[test]
    fn missing_category_is_corrupt() {
        // Registry with no CATEGORY at all.
        let mut reg = Registry::new();
        reg.insert("FAKE".to_owned(), Entry::Integer(1));
        let bytes = save(&reg);
        let r = load(&bytes);
        assert_eq!(r.err, Some(Error::Corrupt));
    }

    #[test]
    fn all_known_categories_accepted() {
        for cat in VALID_CATEGORIES {
            let bytes = build_minimal(cat);
            let r = load(&bytes);
            assert!(
                r.err.is_none(),
                "category {cat:?} rejected: {:?}",
                r.err
            );
            assert_eq!(get_string(&r.sfo, "CATEGORY", ""), *cat);
        }
    }

    // -- Accessor semantics ----------------------------------------

    #[test]
    fn get_string_returns_default_for_integer() {
        let mut reg = Registry::new();
        reg.insert("N".to_owned(), Entry::Integer(7));
        assert_eq!(get_string(&reg, "N", "fallback"), "fallback");
    }

    #[test]
    fn get_integer_returns_default_for_string() {
        let mut reg = Registry::new();
        reg.insert(
            "S".to_owned(),
            Entry::String { value: "hi".to_owned(), max_size: 4 },
        );
        assert_eq!(get_integer(&reg, "S", 99), 99);
    }

    #[test]
    fn get_string_returns_default_for_missing_key() {
        let reg = Registry::new();
        assert_eq!(get_string(&reg, "MISSING", "x"), "x");
    }

    // -- Structural corruption -------------------------------------

    #[test]
    fn truncated_key_table_is_corrupt() {
        let bytes = build_minimal("HG");
        let truncated = &bytes[..bytes.len() - 5]; // chop tail
        let r = load(truncated);
        assert_eq!(r.err, Some(Error::Corrupt));
    }

    #[test]
    fn header_with_inverted_offsets_is_corrupt() {
        // off_key_table > off_data_table → corrupt
        let mut bytes = build_minimal("HG");
        // Swap off_key_table and off_data_table so key > data
        let off_k = read_u32_le(&bytes, 8).unwrap();
        let off_d = read_u32_le(&bytes, 12).unwrap();
        bytes[8..12].copy_from_slice(&off_d.to_le_bytes());
        bytes[12..16].copy_from_slice(&off_k.to_le_bytes());
        let r = load(&bytes);
        assert_eq!(r.err, Some(Error::Corrupt));
    }

    #[test]
    fn entry_with_param_len_greater_than_max_is_corrupt() {
        let bytes = build_minimal("HG");
        let mut mutated = bytes.clone();
        // First def entry starts at offset 20 (HEADER_SIZE). param_len at +4, param_max at +8.
        // Set param_len > param_max
        mutated[20 + 4..20 + 8].copy_from_slice(&100u32.to_le_bytes());
        mutated[20 + 8..20 + 12].copy_from_slice(&4u32.to_le_bytes());
        let r = load(&mutated);
        assert_eq!(r.err, Some(Error::Corrupt));
    }

    // -- Binary format stability -----------------------------------

    #[test]
    fn minimal_psf_binary_layout_is_stable() {
        // Sanity: a minimal PSF with one CATEGORY="HG" entry has a known
        // fixed size: header (20) + 1 def (16) + "CATEGORY\0" (9) +
        // padding to 4-byte align (3) + data slot (4) = 52 bytes.
        let bytes = build_minimal("HG");
        assert_eq!(bytes.len(), 52);
        // Magic
        assert_eq!(&bytes[0..4], &MAGIC.to_le_bytes());
        // Version
        assert_eq!(&bytes[4..8], &VERSION.to_le_bytes());
        // off_key_table = 20 + 16 = 36
        assert_eq!(read_u32_le(&bytes, 8), Some(36));
        // off_data_table = 36 + align4(9) = 36 + 12 = 48
        assert_eq!(read_u32_le(&bytes, 12), Some(48));
        // entries_num = 1
        assert_eq!(read_u32_le(&bytes, 16), Some(1));
        // Key at offset 36
        assert_eq!(&bytes[36..44], b"CATEGORY");
        assert_eq!(bytes[44], 0);
    }

    #[test]
    fn save_produces_deterministic_output() {
        let mut reg = Registry::new();
        reg.insert("CATEGORY".to_owned(), Entry::String { value: "HG".to_owned(), max_size: 4 });
        reg.insert("TITLE_ID".to_owned(), Entry::String { value: "BLES01234".to_owned(), max_size: 16 });
        reg.insert("PARENTAL_LEVEL".to_owned(), Entry::Integer(2));

        let a = save(&reg);
        let b = save(&reg);
        assert_eq!(a, b, "save is deterministic");
    }
}
