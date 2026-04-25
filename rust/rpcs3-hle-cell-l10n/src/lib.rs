//! `rpcs3-hle-cell-l10n` — Rust port of `rpcs3/Emu/Cell/Modules/cellL10n.cpp`.
//!
//! PS3 localization PRX HLE — text codepage conversion (UTF-8 ↔ SJIS, GBK,
//! BIG5, EUC-JP/KR/CN, ISO-8859 family, Shift-JIS variants, UCS-2/4, UTF-16/32,
//! HZ, ARIB) plus Shift-JIS half/full-width folding and kuten-2-jis tables.
//!
//! The real C++ module delegates most conversions to `simdutf` and a hand-
//! rolled SJIS tables layer; porting the full implementation to no_std would
//! pull in a megabyte of codepoint tables. This first pass freezes the
//! observable ABI:
//!
//! - `L10nResult` discriminants (cpp header:8..12).
//! - Detection bitmask flags (`L10N_STR_*`, cpp header:17..24).
//! - `CodePages` enum values (cpp header:30..90) with the alias collapses
//!   (`L10N_SHIFT_JIS = L10N_CODEPAGE_932`, `L10N_GBK = L10N_CODEPAGE_936`,
//!   `L10N_BIG5 = L10N_CODEPAGE_950`, `L10N_JIS = L10N_ISO_2022_JP`,
//!   `L10N_MUSIC_SHIFT_JIS = L10N_RIS_506`).
//! - UTF-16 surrogate masks.
//! - 165 ENTRY_POINTS preserved in REG_FUNC order.
#![no_std]
extern crate alloc;

use rpcs3_emu::CellError;
use rpcs3_emu_types as rpcs3_emu;

pub const CELL_OK: u32 = 0;

// L10nResult (cpp header:8..12) — plain enum, zero-based.
pub const CONVERSION_OK: u32 = 0;
pub const SRC_ILLEGAL: u32 = 1;
pub const DST_EXHAUSTED: u32 = 2;
pub const CONVERTER_UNKNOWN: u32 = 3;

// Detection result bitmasks (cpp header:17..24).
pub const L10N_STR_UNKNOWN: u32 = 1 << 0;
pub const L10N_STR_ASCII: u32 = 1 << 1;
pub const L10N_STR_JIS: u32 = 1 << 2;
pub const L10N_STR_EUCJP: u32 = 1 << 3;
pub const L10N_STR_SJIS: u32 = 1 << 4;
pub const L10N_STR_UTF8: u32 = 1 << 5;
pub const L10N_STR_ILLEGAL: u32 = 1 << 16;
pub const L10N_STR_ERROR: u32 = 1 << 17;

// CodePages (cpp header:30..90) — plain sequential enum with aliases.
pub const L10N_UTF8: u32 = 0;
pub const L10N_UTF16: u32 = 1;
pub const L10N_UTF32: u32 = 2;
pub const L10N_UCS2: u32 = 3;
pub const L10N_UCS4: u32 = 4;
pub const L10N_ISO_8859_1: u32 = 5;
pub const L10N_ISO_8859_2: u32 = 6;
pub const L10N_ISO_8859_3: u32 = 7;
pub const L10N_ISO_8859_4: u32 = 8;
pub const L10N_ISO_8859_5: u32 = 9;
pub const L10N_ISO_8859_6: u32 = 10;
pub const L10N_ISO_8859_7: u32 = 11;
pub const L10N_ISO_8859_8: u32 = 12;
pub const L10N_ISO_8859_9: u32 = 13;
pub const L10N_ISO_8859_10: u32 = 14;
pub const L10N_ISO_8859_11: u32 = 15;
pub const L10N_ISO_8859_13: u32 = 16;
pub const L10N_ISO_8859_14: u32 = 17;
pub const L10N_ISO_8859_15: u32 = 18;
pub const L10N_ISO_8859_16: u32 = 19;
pub const L10N_CODEPAGE_437: u32 = 20;
pub const L10N_CODEPAGE_850: u32 = 21;
pub const L10N_CODEPAGE_863: u32 = 22;
pub const L10N_CODEPAGE_866: u32 = 23;
pub const L10N_CODEPAGE_932: u32 = 24;
pub const L10N_SHIFT_JIS: u32 = L10N_CODEPAGE_932;
pub const L10N_CODEPAGE_936: u32 = 25;
pub const L10N_GBK: u32 = L10N_CODEPAGE_936;
pub const L10N_CODEPAGE_949: u32 = 26;
pub const L10N_UHC: u32 = L10N_CODEPAGE_949;
pub const L10N_CODEPAGE_950: u32 = 27;
pub const L10N_BIG5: u32 = L10N_CODEPAGE_950;
pub const L10N_CODEPAGE_1251: u32 = 28;
pub const L10N_CODEPAGE_1252: u32 = 29;
pub const L10N_EUC_CN: u32 = 30;
pub const L10N_EUC_JP: u32 = 31;
pub const L10N_EUC_KR: u32 = 32;
pub const L10N_ISO_2022_JP: u32 = 33;
pub const L10N_JIS: u32 = L10N_ISO_2022_JP;
pub const L10N_ARIB: u32 = 34;
pub const L10N_HZ: u32 = 35;
pub const L10N_GB18030: u32 = 36;
pub const L10N_RIS_506: u32 = 37;
pub const L10N_MUSIC_SHIFT_JIS: u32 = L10N_RIS_506;
// FW 3.10 and below
pub const L10N_CODEPAGE_852: u32 = 38;
pub const L10N_CODEPAGE_1250: u32 = 39;
pub const L10N_CODEPAGE_737: u32 = 40;
pub const L10N_CODEPAGE_1253: u32 = 41;
pub const L10N_CODEPAGE_857: u32 = 42;
pub const L10N_CODEPAGE_1254: u32 = 43;
pub const L10N_CODEPAGE_775: u32 = 44;
pub const L10N_CODEPAGE_1257: u32 = 45;
pub const L10N_CODEPAGE_855: u32 = 46;
pub const L10N_CODEPAGE_858: u32 = 47;
pub const L10N_CODEPAGE_860: u32 = 48;
pub const L10N_CODEPAGE_861: u32 = 49;
pub const L10N_CODEPAGE_865: u32 = 50;
pub const L10N_CODEPAGE_869: u32 = 51;
/// Sentinel one past the last real codepage id (cpp header:89).
pub const L10N_CODE_END: u32 = 52;

// UTF-16 surrogate helpers (cpp header:93..99).
pub const UTF16_SURROGATES_MASK1: u16 = 0xf800;
pub const UTF16_SURROGATES_MASK2: u16 = 0xfc00;
pub const UTF16_SURROGATES: u16 = 0xd800;
pub const UTF16_HIGH_SURROGATES: u16 = 0xd800;
pub const UTF16_LOW_SURROGATES: u16 = 0xdc00;

/// Registration-order list of the 165 REG_FUNC entries (cpp:2689..2853).
/// Preserved byte-exact so FNID-based PRX lookups resolve identically.
pub const ENTRY_POINTS: &[&str] = &[
    "UCS2toEUCJP",
    "l10n_convert",
    "UCS2toUTF32",
    "jis2kuten",
    "UTF8toGB18030",
    "JISstoUTF8s",
    "SjisZen2Han",
    "ToSjisLower",
    "UCS2toGB18030",
    "HZstoUCS2s",
    "UCS2stoHZs",
    "UCS2stoSJISs",
    "kuten2eucjp",
    "sjis2jis",
    "EUCKRstoUCS2s",
    "UHCstoEUCKRs",
    "jis2sjis",
    "jstrnchk",
    "L10nConvert",
    "EUCCNstoUTF8s",
    "GBKstoUCS2s",
    "eucjphan2zen",
    "ToSjisHira",
    "GBKtoUCS2",
    "eucjp2jis",
    "UTF32stoUTF8s",
    "sjishan2zen",
    "UCS2toSBCS",
    "UTF8stoGBKs",
    "UTF8toUCS2",
    "UCS2stoUTF8s",
    "EUCKRstoUTF8s",
    "UTF16stoUTF32s",
    "UTF8toEUCKR",
    "UTF16toUTF8",
    "ARIBstoUTF8s",
    "SJISstoUTF8s",
    "sjiszen2han",
    "ToEucJpLower",
    "MSJIStoUTF8",
    "UCS2stoMSJISs",
    "EUCJPtoUTF8",
    "eucjp2sjis",
    "ToEucJpHira",
    "UHCstoUCS2s",
    "ToEucJpKata",
    "HZstoUTF8s",
    "UTF8toMSJIS",
    "BIG5toUTF8",
    "EUCJPstoSJISs",
    "UTF8stoBIG5s",
    "UTF16stoUCS2s",
    "UCS2stoGB18030s",
    "EUCJPtoSJIS",
    "EUCJPtoUCS2",
    "UCS2stoGBKs",
    "EUCKRtoUHC",
    "UCS2toSJIS",
    "MSJISstoUTF8s",
    "EUCJPstoUTF8s",
    "UCS2toBIG5",
    "UTF8stoEUCKRs",
    "UHCstoUTF8s",
    "GB18030stoUCS2s",
    "SJIStoUTF8",
    "JISstoSJISs",
    "UTF8toUTF16",
    "UTF8stoMSJISs",
    "EUCKRtoUTF8",
    "SjisHan2Zen",
    "UCS2toUTF16",
    "UCS2toMSJIS",
    "sjis2kuten",
    "UCS2toUHC",
    "UTF32toUCS2",
    "ToSjisUpper",
    "UTF8toEUCJP",
    "UCS2stoEUCJPs",
    "UTF16toUCS2",
    "UCS2stoUTF16s",
    "UCS2stoEUCCNs",
    "SBCSstoUTF8s",
    "SJISstoJISs",
    "SBCStoUTF8",
    "UTF8toUTF32",
    "jstrchk",
    "UHCtoEUCKR",
    "kuten2jis",
    "UTF8toEUCCN",
    "EUCCNtoUTF8",
    "EucJpZen2Han",
    "UTF32stoUTF16s",
    "GBKtoUTF8",
    "ToEucJpUpper",
    "UCS2stoJISs",
    "UTF8stoGB18030s",
    "EUCKRstoUHCs",
    "UTF8stoUTF32s",
    "UTF8stoEUCCNs",
    "EUCJPstoUCS2s",
    "UHCtoUCS2",
    "L10nConvertStr",
    "GBKstoUTF8s",
    "UTF8toUHC",
    "UTF32toUTF8",
    "sjis2eucjp",
    "UCS2toEUCCN",
    "UTF8stoUHCs",
    "EUCKRtoUCS2",
    "UTF32toUTF16",
    "EUCCNstoUCS2s",
    "SBCSstoUCS2s",
    "UTF8stoJISs",
    "ToSjisKata",
    "jis2eucjp",
    "BIG5toUCS2",
    "UCS2toGBK",
    "UTF16toUTF32",
    "l10n_convert_str",
    "EUCJPstoJISs",
    "UTF8stoARIBs",
    "JISstoEUCJPs",
    "EucJpHan2Zen",
    "isEucJpKigou",
    "UCS2toUTF8",
    "GB18030toUCS2",
    "UHCtoUTF8",
    "MSJIStoUCS2",
    "UTF8toGBK",
    "kuten2sjis",
    "UTF8toSBCS",
    "SJIStoUCS2",
    "eucjpzen2han",
    "UCS2stoARIBs",
    "isSjisKigou",
    "UTF8stoEUCJPs",
    "UCS2toEUCKR",
    "SBCStoUCS2",
    "MSJISstoUCS2s",
    "l10n_get_converter",
    "GB18030stoUTF8s",
    "SJISstoEUCJPs",
    "UTF32stoUCS2s",
    "BIG5stoUTF8s",
    "EUCCNtoUCS2",
    "UTF8stoSBCSs",
    "UCS2stoEUCKRs",
    "UTF8stoSJISs",
    "UTF8stoHZs",
    "eucjp2kuten",
    "UTF8toBIG5",
    "UTF16stoUTF8s",
    "JISstoUCS2s",
    "GB18030toUTF8",
    "UTF8toSJIS",
    "ARIBstoUCS2s",
    "UCS2stoUTF32s",
    "UCS2stoSBCSs",
    "UCS2stoBIG5s",
    "UCS2stoUHCs",
    "SJIStoEUCJP",
    "UTF8stoUTF16s",
    "SJISstoUCS2s",
    "BIG5stoUCS2s",
    "UTF8stoUCS2s",
];

/// Check whether a UTF-16 code unit is the high half of a surrogate pair.
#[must_use]
pub const fn is_utf16_high_surrogate(u: u16) -> bool {
    (u & UTF16_SURROGATES_MASK2) == UTF16_HIGH_SURROGATES
}

/// Check whether a UTF-16 code unit is the low half of a surrogate pair.
#[must_use]
pub const fn is_utf16_low_surrogate(u: u16) -> bool {
    (u & UTF16_SURROGATES_MASK2) == UTF16_LOW_SURROGATES
}

/// Dispatcher for the L10n PRX surface. Real conversion is not implemented —
/// every call bumps a slot and returns `CONVERSION_OK` so the binding layer
/// can be wired up and traced.
pub struct L10nHle {
    pub call_counts: [u64; 165],
}

impl L10nHle {
    pub const fn new() -> Self {
        Self { call_counts: [0; 165] }
    }

    pub fn dispatch(&mut self, index: usize) -> Result<u32, CellError> {
        if index >= ENTRY_POINTS.len() {
            return Err(CellError(0x8001_0000));
        }
        self.call_counts[index] = self.call_counts[index].saturating_add(1);
        Ok(CONVERSION_OK)
    }

    pub fn dispatch_by_name(&mut self, name: &str) -> Result<u32, CellError> {
        match ENTRY_POINTS.iter().position(|&e| e == name) {
            Some(i) => self.dispatch(i),
            None => Err(CellError(0x8001_0000)),
        }
    }
}

impl Default for L10nHle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_count_matches_cpp() {
        assert_eq!(ENTRY_POINTS.len(), 165, "REG_FUNC count cpp:2689..2853");
    }

    #[test]
    fn first_and_last_entry() {
        assert_eq!(ENTRY_POINTS[0], "UCS2toEUCJP");
        assert_eq!(ENTRY_POINTS[164], "UTF8stoUCS2s");
    }

    #[test]
    fn l10n_result_values() {
        assert_eq!(CONVERSION_OK, 0);
        assert_eq!(SRC_ILLEGAL, 1);
        assert_eq!(DST_EXHAUSTED, 2);
        assert_eq!(CONVERTER_UNKNOWN, 3);
    }

    #[test]
    fn detection_flag_bits() {
        assert_eq!(L10N_STR_UNKNOWN, 0x1);
        assert_eq!(L10N_STR_ASCII, 0x2);
        assert_eq!(L10N_STR_UTF8, 0x20);
        assert_eq!(L10N_STR_ILLEGAL, 0x1_0000);
        assert_eq!(L10N_STR_ERROR, 0x2_0000);
    }

    #[test]
    fn codepage_aliases_collapse_correctly() {
        assert_eq!(L10N_SHIFT_JIS, L10N_CODEPAGE_932);
        assert_eq!(L10N_GBK, L10N_CODEPAGE_936);
        assert_eq!(L10N_UHC, L10N_CODEPAGE_949);
        assert_eq!(L10N_BIG5, L10N_CODEPAGE_950);
        assert_eq!(L10N_JIS, L10N_ISO_2022_JP);
        assert_eq!(L10N_MUSIC_SHIFT_JIS, L10N_RIS_506);
    }

    #[test]
    fn codepage_ordering_frozen() {
        // spot-check the sequential layout: alphabet sanity + alias slots.
        assert_eq!(L10N_UTF8, 0);
        assert_eq!(L10N_CODEPAGE_437, 20);
        assert_eq!(L10N_CODEPAGE_932, 24);
        assert_eq!(L10N_ISO_2022_JP, 33);
        assert_eq!(L10N_RIS_506, 37);
        assert_eq!(L10N_CODEPAGE_869, 51);
        assert_eq!(L10N_CODE_END, 52);
    }

    #[test]
    fn surrogate_masks_detect_pairs() {
        assert!(is_utf16_high_surrogate(0xd800));
        assert!(is_utf16_high_surrogate(0xdbff));
        assert!(!is_utf16_high_surrogate(0xdc00));
        assert!(is_utf16_low_surrogate(0xdc00));
        assert!(is_utf16_low_surrogate(0xdfff));
        assert!(!is_utf16_low_surrogate(0xd800));
    }

    #[test]
    fn dispatch_returns_ok_and_bumps() {
        let mut hle = L10nHle::new();
        assert_eq!(hle.dispatch(0).unwrap(), CONVERSION_OK);
        assert_eq!(hle.call_counts[0], 1);
        hle.dispatch_by_name("UTF8toUCS2").unwrap();
        let idx = ENTRY_POINTS.iter().position(|&e| e == "UTF8toUCS2").unwrap();
        assert_eq!(hle.call_counts[idx], 1);
    }

    #[test]
    fn dispatch_oob_and_unknown_name() {
        let mut hle = L10nHle::new();
        assert_eq!(hle.dispatch(165).unwrap_err().0, 0x8001_0000);
        assert_eq!(hle.dispatch_by_name("bogus").unwrap_err().0, 0x8001_0000);
    }
}
