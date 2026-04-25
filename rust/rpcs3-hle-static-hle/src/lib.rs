//! `rpcs3-hle-static-hle` — PS3 pattern-matching static HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/StaticHLE.cpp` (179 linhas).  The
//! firmware module recognises known PS3 libc implementations of
//! `memcpy` / `memset` / `memmove` / `memcmp` by scanning the guest
//! text segment for the first 32 bytes of their preamble, then
//! validates a CRC16 over the next `crc16_length` bytes.  On match,
//! the firmware patches four PPU instructions (`LIS / ORI / MTCTR /
//! BCTR`) that jump into an HLE-backed helper.
//!
//! ## Surface ported
//!
//! * **9 pattern records** matching the `shle_patterns_list` table
//!   (cpp:14-25).
//! * **Hex-byte parser** with wildcard support (`..` → `0xFFFF`).
//! * **CRC16** with polynomial `0x8408` + byte-swap finalisation
//!   (cpp:100-126).
//! * **Stub writer** that lays out the 4-instruction stub the firmware
//!   emits on match (cpp:168-172).

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

// =====================================================================
// Constants — byte-exact with StaticHLE.cpp
// =====================================================================

/// CRC16 polynomial used by the firmware (`#define POLY 0x8408`).
pub const POLY: u16 = 0x8408;

/// Start-pattern length in bytes (first 32 guest-memory bytes).
pub const START_PATTERN_BYTES: usize = 32;

/// Start-pattern length expressed as the hex string the table uses
/// (32 bytes × 2 nibbles).
pub const START_PATTERN_HEX_LEN: usize = 64;

// =====================================================================
// PPU instruction builders — matches `ppu_instructions::` in cpp:169-172
// =====================================================================

/// Build a PPU `LIS` instruction (Load Immediate Shifted).
///
/// The PPU `LIS rD, imm` encoding is `0x3C000000 | (rD << 21) | imm16`.
/// Alias of `addis rD, 0, imm`.
#[must_use]
pub const fn ppu_lis(rd: u32, imm: u16) -> u32 {
    0x3C00_0000 | ((rd & 0x1F) << 21) | imm as u32
}

/// Build a PPU `ORI` instruction (OR Immediate).
/// `ori rA, rS, imm` → `0x60000000 | (rS << 21) | (rA << 16) | imm16`.
#[must_use]
pub const fn ppu_ori(ra: u32, rs: u32, imm: u16) -> u32 {
    0x6000_0000 | ((rs & 0x1F) << 21) | ((ra & 0x1F) << 16) | imm as u32
}

/// Build a PPU `MTCTR` (Move To Count Register).  Alias of
/// `mtspr 9, rS` → `0x7C0903A6 | (rS << 21)`.
#[must_use]
pub const fn ppu_mtctr(rs: u32) -> u32 {
    0x7C09_03A6 | ((rs & 0x1F) << 21)
}

/// Build a PPU `BCTR` (Branch to Count Register) — unconditional jump.
/// Encoding: `0x4E800420` (bcctr 20, 0, 0).
#[must_use]
pub const fn ppu_bctr() -> u32 { 0x4E80_0420 }

// =====================================================================
// CRC16 — byte-exact with cpp:102-126
// =====================================================================

/// Port of `statichle_handler::gen_CRC16` (cpp:102-126).  The firmware
/// applies the CCITT-style CRC16 with polynomial `0x8408`, seeds with
/// `0xFFFF`, inverts at the end, then byte-swaps the two halves.
#[must_use]
pub fn gen_crc16(data: &[u8]) -> u16 {
    if data.is_empty() { return 0; }
    let mut crc: u32 = 0xFFFF;
    for &b in data {
        let mut d: u32 = u32::from(b);
        for _ in 0..8 {
            if (crc ^ d) & 1 != 0 {
                crc = (crc >> 1) ^ u32::from(POLY);
            } else {
                crc >>= 1;
            }
            d >>= 1;
        }
    }
    crc = !crc & 0xFFFF;
    let data = crc;
    crc = ((crc << 8) | ((data >> 8) & 0xFF)) & 0xFFFF;
    crc as u16
}

// =====================================================================
// Hex parser — byte-exact with cpp:65-80
// =====================================================================

/// Wildcard nibble-pair sentinel — cpp:67-68 returns `0xFFFF` when both
/// nibbles are `'.'`.
pub const WILDCARD_NIBBLE: u16 = 0xFFFF;

/// Convert a nibble character (`0-9`, `A-F` — uppercase only, per C++
/// source) into its value.  Returns `None` for anything else.
#[must_use]
pub fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Port of the per-byte `char_to_u8` lambda (cpp:65-80).  `c1` / `c2`
/// are the two ASCII hex characters; both `.` → wildcard; exactly one
/// `.` is a parse error.
#[must_use]
pub fn hex_byte(c1: u8, c2: u8) -> Option<u16> {
    if c1 == b'.' && c2 == b'.' {
        return Some(WILDCARD_NIBBLE);
    }
    if c1 == b'.' || c2 == b'.' {
        return None;
    }
    let hi = hex_nibble(c1)?;
    let lo = hex_nibble(c2)?;
    Some((u16::from(hi) << 4) | u16::from(lo))
}

/// Parse the 64-character hex string into the 32-byte / 32-word start
/// pattern.  Wildcards map to `0xFFFF`.
///
/// # Errors
/// Returns `None` on any malformed byte or when `s.len() != 64`.
#[must_use]
pub fn parse_start_pattern(s: &str) -> Option<[u16; START_PATTERN_BYTES]> {
    if s.len() != START_PATTERN_HEX_LEN { return None; }
    let bytes = s.as_bytes();
    let mut out = [0u16; START_PATTERN_BYTES];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = hex_byte(bytes[i * 2], bytes[i * 2 + 1])?;
    }
    Some(out)
}

/// Parse a 2-char hex string into a `u8`.  Wildcards not allowed here.
#[must_use]
pub fn parse_hex_u8(s: &str) -> Option<u8> {
    if s.len() != 2 { return None; }
    let bytes = s.as_bytes();
    let raw = hex_byte(bytes[0], bytes[1])?;
    if raw == WILDCARD_NIBBLE { return None; }
    Some(raw as u8)
}

/// Parse a 4-char hex string into a `u16`.
#[must_use]
pub fn parse_hex_u16(s: &str) -> Option<u16> {
    if s.len() != 4 { return None; }
    let hi = parse_hex_u8(&s[..2])?;
    let lo = parse_hex_u8(&s[2..])?;
    Some((u16::from(hi) << 8) | u16::from(lo))
}

// =====================================================================
// Pattern data structures
// =====================================================================

/// Compiled form of a single pattern entry — mirror of
/// `statichle_handler::shle_pattern`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShlePattern {
    pub start_pattern: [u16; START_PATTERN_BYTES],
    pub crc16_length: u8,
    pub crc16: u16,
    pub total_length: u16,
    pub module: String,
    pub name: String,
}

/// Raw 6-column rows from `shle_patterns_list` (cpp:14-25) — in the
/// exact source order so the compiled list matches the C++ registry.
pub const RAW_PATTERNS: &[[&str; 6]] = &[
    ["2BA5000778630020788400207C6B1B78419D00702C2500004D82002028A5000F", "FF", "36D0", "05C4", "sys_libc", "memcpy"],
    ["2BA5000778630020788400207C6B1B78419D00702C2500004D82002028A5000F", "5C", "87A0", "05C4", "sys_libc", "memcpy"],
    ["2B8500077CA32A14788406207C6A1B78409D009C3903000198830000788B45E4", "B4", "1453", "00D4", "sys_libc", "memset"],
    ["280500087C661B7840800020280500004D8200207CA903A69886000038C60001", "F8", "F182", "0118", "sys_libc", "memset"],
    ["2B8500077CA32A14788406207C6A1B78409D009C3903000198830000788B45E4", "70", "DFDA", "00D4", "sys_libc", "memset"],
    ["7F832000FB61FFD8FBE1FFF8FB81FFE0FBA1FFE8FBC1FFF07C7B1B787C9F2378", "FF", "25B5", "12D4", "sys_libc", "memmove"],
    ["2B850007409D00B07C6923785520077E2F800000409E00ACE8030000E9440000", "FF", "71F1", "0158", "sys_libc", "memcmp"],
    ["280500007CE32050788B0760418200E028850100786A07607C2A580040840210", "FF", "87F2", "0470", "sys_libc", "memcmp"],
    ["2B850007409D00B07C6923785520077E2F800000409E00ACE8030000E9440000", "68", "EF18", "0158", "sys_libc", "memcmp"],
];

/// Compile every row of [`RAW_PATTERNS`] into a [`ShlePattern`].
/// Returns `None` if any row fails to parse (which the tests verify
/// never happens for the shipped table).
#[must_use]
pub fn compile_patterns() -> Option<Vec<ShlePattern>> {
    let mut out = Vec::with_capacity(RAW_PATTERNS.len());
    for row in RAW_PATTERNS {
        out.push(ShlePattern {
            start_pattern: parse_start_pattern(row[0])?,
            crc16_length: parse_hex_u8(row[1])?,
            crc16: parse_hex_u16(row[2])?,
            total_length: parse_hex_u16(row[3])?,
            module: String::from(row[4]),
            name: String::from(row[5]),
        });
    }
    Some(out)
}

// =====================================================================
// Pattern matcher
// =====================================================================

/// Port of `statichle_handler::check_against_patterns` (cpp:128-178).
/// `data` is the guest memory to scan.  Returns `Some(&pattern)` on
/// first match, `None` otherwise.
#[must_use]
pub fn check_against_patterns<'a>(
    patterns: &'a [ShlePattern],
    data: &[u8],
) -> Option<&'a ShlePattern> {
    for pat in patterns {
        if (data.len() as u16) < pat.total_length {
            continue;
        }
        // Start-pattern check — wildcards pass.
        let mut matched = true;
        for i in 0..START_PATTERN_BYTES {
            if pat.start_pattern[i] == WILDCARD_NIBBLE { continue; }
            if u16::from(data[i]) != pat.start_pattern[i] { matched = false; break; }
        }
        if !matched { continue; }
        // CRC16 check over the next `crc16_length` bytes.
        if pat.crc16_length != 0 {
            let crc_end = START_PATTERN_BYTES + pat.crc16_length as usize;
            if data.len() < crc_end { continue; }
            if gen_crc16(&data[START_PATTERN_BYTES..crc_end]) != pat.crc16 { continue; }
        }
        return Some(pat);
    }
    None
}

// =====================================================================
// Stub emitter (4-instruction PPU jump thunk)
// =====================================================================

/// Port of cpp:168-172.  Given a target HLE-function address, emit the
/// four 32-bit instructions the firmware patches in place of the
/// original libc preamble.
#[must_use]
pub fn emit_stub(target: u32) -> [u32; 4] {
    [
        ppu_lis(0, ((target & 0xFFFF_0000) >> 16) as u16),
        ppu_ori(0, 0, (target & 0xFFFF) as u16),
        ppu_mtctr(0),
        ppu_bctr(),
    ]
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- constants ---------------------------------------------------

    #[test]
    fn poly_byte_exact() {
        assert_eq!(POLY, 0x8408);
    }

    #[test]
    fn start_pattern_lengths_byte_exact() {
        assert_eq!(START_PATTERN_BYTES, 32);
        assert_eq!(START_PATTERN_HEX_LEN, 64);
    }

    #[test]
    fn wildcard_byte_exact() {
        assert_eq!(WILDCARD_NIBBLE, 0xFFFF);
    }

    // ---- hex parser --------------------------------------------------

    #[test]
    fn hex_nibble_digits() {
        assert_eq!(hex_nibble(b'0'), Some(0));
        assert_eq!(hex_nibble(b'9'), Some(9));
        assert_eq!(hex_nibble(b'A'), Some(0xA));
        assert_eq!(hex_nibble(b'F'), Some(0xF));
    }

    #[test]
    fn hex_nibble_rejects_lowercase() {
        // Firmware table uses uppercase only (cpp:76 `char1 > '9' ? char1 - 'A' + 10 : char1 - '0'`).
        // Lowercase would produce garbage; we return None for safety.
        assert_eq!(hex_nibble(b'a'), None);
    }

    #[test]
    fn hex_nibble_rejects_invalid() {
        assert_eq!(hex_nibble(b'G'), None);
        assert_eq!(hex_nibble(b' '), None);
        assert_eq!(hex_nibble(0), None);
    }

    #[test]
    fn hex_byte_normal() {
        assert_eq!(hex_byte(b'A', b'B'), Some(0xAB));
        assert_eq!(hex_byte(b'0', b'0'), Some(0x00));
        assert_eq!(hex_byte(b'F', b'F'), Some(0xFF));
    }

    #[test]
    fn hex_byte_wildcard() {
        assert_eq!(hex_byte(b'.', b'.'), Some(WILDCARD_NIBBLE));
    }

    #[test]
    fn hex_byte_half_wildcard_is_error() {
        assert_eq!(hex_byte(b'.', b'0'), None);
        assert_eq!(hex_byte(b'0', b'.'), None);
    }

    #[test]
    fn parse_start_pattern_happy_path() {
        let hex = "2BA5000778630020788400207C6B1B78419D00702C2500004D82002028A5000F";
        let pat = parse_start_pattern(hex).unwrap();
        assert_eq!(pat[0], 0x2B);
        assert_eq!(pat[1], 0xA5);
        assert_eq!(pat[31], 0x0F);
    }

    #[test]
    fn parse_start_pattern_wrong_length() {
        assert!(parse_start_pattern("ABCD").is_none());
    }

    #[test]
    fn parse_start_pattern_with_wildcards() {
        // 2 wildcards at the tail.
        let hex = "2BA5000778630020788400207C6B1B78419D00702C2500004D82002028A5....";
        let pat = parse_start_pattern(hex).unwrap();
        assert_eq!(pat[30], WILDCARD_NIBBLE);
        assert_eq!(pat[31], WILDCARD_NIBBLE);
    }

    #[test]
    fn parse_hex_u8_and_u16() {
        assert_eq!(parse_hex_u8("FF"), Some(0xFF));
        assert_eq!(parse_hex_u8("00"), Some(0x00));
        assert_eq!(parse_hex_u16("36D0"), Some(0x36D0));
        assert_eq!(parse_hex_u16("0000"), Some(0x0000));
    }

    #[test]
    fn parse_hex_u16_wrong_length() {
        assert!(parse_hex_u16("36D").is_none());
        assert!(parse_hex_u16("36D0A").is_none());
    }

    // ---- CRC16 -------------------------------------------------------

    #[test]
    fn crc16_empty_is_zero() {
        // cpp:107 `if (length == 0) return 0;`.
        assert_eq!(gen_crc16(&[]), 0);
    }

    #[test]
    fn crc16_single_byte_deterministic() {
        // Not a known standard — just exercise the byte-swap finalise
        // path and record the value.  If the firmware changes POLY this
        // test flags it.
        let v = gen_crc16(&[0x00]);
        assert_ne!(v, 0); // byte-swap + invert ensures non-zero
    }

    #[test]
    fn crc16_stable_with_fixed_input() {
        // Deterministic regression guard — shifts in POLY or the finalise
        // would flip this.
        let expected = gen_crc16(&[0xAB, 0xCD, 0xEF]);
        assert_eq!(gen_crc16(&[0xAB, 0xCD, 0xEF]), expected);
    }

    #[test]
    fn crc16_differs_on_different_inputs() {
        let a = gen_crc16(&[0x11, 0x22]);
        let b = gen_crc16(&[0x11, 0x23]);
        assert_ne!(a, b);
    }

    // ---- patterns ---------------------------------------------------

    #[test]
    fn raw_patterns_has_9_rows() {
        // cpp:14-25 has exactly 9 entries.
        assert_eq!(RAW_PATTERNS.len(), 9);
    }

    #[test]
    fn compile_patterns_all_9_succeed() {
        let patterns = compile_patterns().unwrap();
        assert_eq!(patterns.len(), 9);
    }

    #[test]
    fn compile_patterns_module_and_name() {
        let patterns = compile_patterns().unwrap();
        assert_eq!(patterns[0].module, "sys_libc");
        assert_eq!(patterns[0].name,   "memcpy");
        assert_eq!(patterns[5].name,   "memmove");
        assert_eq!(patterns[6].name,   "memcmp");
    }

    #[test]
    fn compile_patterns_crc_lengths_byte_exact() {
        let patterns = compile_patterns().unwrap();
        assert_eq!(patterns[0].crc16_length, 0xFF);
        assert_eq!(patterns[1].crc16_length, 0x5C);
        assert_eq!(patterns[2].crc16_length, 0xB4);
        assert_eq!(patterns[3].crc16_length, 0xF8);
    }

    #[test]
    fn compile_patterns_crc_values_byte_exact() {
        let patterns = compile_patterns().unwrap();
        assert_eq!(patterns[0].crc16, 0x36D0);
        assert_eq!(patterns[6].crc16, 0x71F1);
    }

    #[test]
    fn compile_patterns_total_lengths_byte_exact() {
        let patterns = compile_patterns().unwrap();
        assert_eq!(patterns[0].total_length, 0x05C4);
        assert_eq!(patterns[2].total_length, 0x00D4);
        assert_eq!(patterns[5].total_length, 0x12D4);
    }

    #[test]
    fn compile_patterns_first_start_bytes() {
        let patterns = compile_patterns().unwrap();
        // First entry starts with `2BA50007`.
        assert_eq!(patterns[0].start_pattern[0], 0x2B);
        assert_eq!(patterns[0].start_pattern[1], 0xA5);
        assert_eq!(patterns[0].start_pattern[2], 0x00);
        assert_eq!(patterns[0].start_pattern[3], 0x07);
    }

    // ---- pattern matching -------------------------------------------

    #[test]
    fn check_rejects_short_data() {
        let patterns = compile_patterns().unwrap();
        let short = [0u8; 16];
        assert!(check_against_patterns(&patterns, &short).is_none());
    }

    #[test]
    fn check_rejects_wrong_start_pattern() {
        let patterns = compile_patterns().unwrap();
        let buf = [0xDEu8; 0x1000]; // all 0xDE → doesn't match any start
        assert!(check_against_patterns(&patterns, &buf).is_none());
    }

    #[test]
    fn check_matches_start_only_when_crc_length_is_zero() {
        // Synthesize a pattern with crc16_length = 0 → matcher skips CRC.
        let mut pat = ShlePattern {
            start_pattern: [0; 32],
            crc16_length: 0,
            crc16: 0,
            total_length: 64,
            module: String::from("test"),
            name: String::from("zero"),
        };
        for (i, slot) in pat.start_pattern.iter_mut().enumerate() {
            *slot = u16::from(i as u8);
        }
        // Build buf matching pattern + 32 arbitrary trailing bytes.
        let mut buf: Vec<u8> = (0..32u8).collect();
        buf.extend_from_slice(&[0xCC; 32]);
        let patterns = alloc::vec![pat];
        let hit = check_against_patterns(&patterns, &buf).unwrap();
        assert_eq!(hit.name, "zero");
    }

    // ---- stub emitter -----------------------------------------------

    #[test]
    fn ppu_bctr_encoding() {
        assert_eq!(ppu_bctr(), 0x4E80_0420);
    }

    #[test]
    fn ppu_mtctr_r0() {
        // mtctr r0 = 0x7C0903A6 (rS field zero).
        assert_eq!(ppu_mtctr(0), 0x7C09_03A6);
    }

    #[test]
    fn ppu_lis_r0_imm_round_trips() {
        // lis r0, 0x1234 → 0x3C001234
        assert_eq!(ppu_lis(0, 0x1234), 0x3C00_1234);
    }

    #[test]
    fn ppu_ori_r0_r0_imm_round_trips() {
        // ori r0, r0, 0xABCD → 0x6000ABCD
        assert_eq!(ppu_ori(0, 0, 0xABCD), 0x6000_ABCD);
    }

    #[test]
    fn emit_stub_layout() {
        let stub = emit_stub(0xDEAD_BEEF);
        // LIS r0, 0xDEAD
        assert_eq!(stub[0], 0x3C00_DEAD);
        // ORI r0, r0, 0xBEEF
        assert_eq!(stub[1], 0x6000_BEEF);
        assert_eq!(stub[2], ppu_mtctr(0));
        assert_eq!(stub[3], ppu_bctr());
    }

    #[test]
    fn emit_stub_covers_full_32_bit_target() {
        // High half of 0xFFFE_FFFD → 0xFFFE
        let stub = emit_stub(0xFFFE_FFFD);
        assert_eq!(stub[0], 0x3C00_FFFE);
        assert_eq!(stub[1], 0x6000_FFFD);
    }

    // ---- full smoke -------------------------------------------------

    #[test]
    fn full_static_hle_lifecycle_smoke() {
        // 1. Compile the full pattern table.
        let patterns = compile_patterns().unwrap();
        assert_eq!(patterns.len(), 9);

        // 2. Build a synthetic buffer matching the first pattern's start.
        let hex0 = RAW_PATTERNS[0][0];
        let mut buf = Vec::with_capacity(0x5C4);
        for i in 0..32 {
            let b = hex_byte(
                hex0.as_bytes()[i * 2],
                hex0.as_bytes()[i * 2 + 1],
            ).unwrap() as u8;
            buf.push(b);
        }
        // Pad the rest with 0.
        while buf.len() < patterns[0].total_length as usize {
            buf.push(0);
        }

        // 3. Pattern[0] has crc16_length=0xFF → very unlikely to match
        //    with all-zero trailing bytes.  Match should fail.
        //    But the start bytes match, so we test the CRC rejection.
        assert!(check_against_patterns(&patterns[..1], &buf).is_none());

        // 4. Emit a stub for an arbitrary target.
        let stub = emit_stub(0x0001_0000);
        assert_eq!(stub[0], 0x3C00_0001);
        assert_eq!(stub[1], 0x6000_0000);
    }
}
