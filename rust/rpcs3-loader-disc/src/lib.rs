//! `rpcs3-loader-disc` — Rust port of `rpcs3/Loader/disc.cpp`.
//!
//! RPCS3 asks "what kind of disc is this?" when the user picks a folder.
//! The C++ `get_disc_type` walks the tree looking for a PS3 EBOOT first,
//! falling back to a `SYSTEM.CNF` text file (PS1/PS2 convention). This
//! crate freezes the portable parts: the enum, the SYSTEM.CNF key/value
//! parser, and the PS3-category check (PARAM.SFO CATEGORY must be "DG").
//!
//! Filesystem walking and PARAM.SFO loading themselves live elsewhere
//! (the VFS helpers and `rpcs3-loader-psf`); here we expose pure text
//! parsers + classification so a frontend can reuse them without
//! depending on RPCS3's `Emulator` static.
//!
//! Frozen:
//!
//! - `DiscType` enum (Invalid / Unknown / Ps1 / Ps2 / Ps3) — cpp:5..12.
//! - SYSTEM.CNF keys: `BOOT`, `BOOT2`, `VMODE`, `VER` (cpp:122..139).
//! - "First '=' wins" line split; value trimmed, empty values on non-last
//!   non-blank lines are warnings but not errors (cpp:115..119).
//! - PS3 PARAM.SFO CATEGORY must equal `"DG"` (cpp:60..65).

/// `disc::disc_type` (`disc.h:5..12`). Variants are preserved in order so
/// the `as u32` discriminants match the cpp enum class ordinals.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscType {
    Invalid = 0,
    Unknown = 1,
    Ps1 = 2,
    Ps2 = 3,
    Ps3 = 4,
}

/// CATEGORY value that marks a disc-game PARAM.SFO (cpp:62).
pub const PS3_DISC_CATEGORY: &str = "DG";

/// Parsed SYSTEM.CNF entry. `BOOT`/`BOOT2`/`VMODE`/`VER` are the four
/// keys the cpp cares about; unknown keys are preserved so callers can
/// decide what to do with them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemCnfEntry {
    pub key: String,
    pub value: String,
}

/// Result of classifying a SYSTEM.CNF. Mirrors the cpp enum assignment at
/// cpp:122..131.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemCnfDiscType {
    /// SYSTEM.CNF contained a `BOOT2=` entry → PS2.
    Ps2,
    /// SYSTEM.CNF contained a `BOOT=` entry → PS1.
    Ps1,
    /// Neither — the disc is not a PS1/PS2 game.
    Unknown,
}

/// Checks whether a PARAM.SFO CATEGORY string identifies a PS3 disc game
/// (cpp:60..65).
#[must_use]
pub fn is_ps3_disc_category(category: &str) -> bool {
    category == PS3_DISC_CATEGORY
}

/// Parse a SYSTEM.CNF blob into key/value pairs. Mirrors cpp:97..120.
///
/// Rules:
/// - Split on `\n`.
/// - First `=` wins; everything before is key, everything after is value.
/// - Both key and value are trimmed (leading/trailing whitespace).
/// - Lines without `=` are skipped.
/// - Empty values are still emitted (the cpp merely logs a warning and
///   continues — we surface them so callers can decide).
#[must_use]
pub fn parse_system_cnf(text: &str) -> Vec<SystemCnfEntry> {
    let mut out = Vec::new();
    for line in text.split('\n') {
        if let Some(eq_pos) = line.find('=') {
            let key = trim_cnf(&line[..eq_pos]);
            let value = trim_cnf(&line[eq_pos + 1..]);
            out.push(SystemCnfEntry {
                key: key.to_string(),
                value: value.to_string(),
            });
        }
    }
    out
}

/// Walk a parsed SYSTEM.CNF and classify the disc (cpp:122..131).
#[must_use]
pub fn classify_system_cnf(entries: &[SystemCnfEntry]) -> SystemCnfDiscType {
    let mut result = SystemCnfDiscType::Unknown;
    for e in entries {
        match e.key.as_str() {
            "BOOT2" => result = SystemCnfDiscType::Ps2,
            "BOOT" => {
                // cpp:122 checks BOOT2 first; BOOT alone promotes to PS1.
                // If an entry list has both, the last one wins — the cpp
                // loop does the same (no early-break, just overwrite).
                result = SystemCnfDiscType::Ps1;
            }
            _ => {}
        }
    }
    result
}

fn trim_cnf(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace())
}

/// Convenience: parse + classify in one call. Returns the classified
/// type (never `Ps3`; callers determine PS3 through a different path).
#[must_use]
pub fn classify_text(text: &str) -> SystemCnfDiscType {
    let entries = parse_system_cnf(text);
    classify_system_cnf(&entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disc_type_discriminants() {
        assert_eq!(DiscType::Invalid as u32, 0);
        assert_eq!(DiscType::Unknown as u32, 1);
        assert_eq!(DiscType::Ps1 as u32, 2);
        assert_eq!(DiscType::Ps2 as u32, 3);
        assert_eq!(DiscType::Ps3 as u32, 4);
    }

    #[test]
    fn ps3_category_is_dg() {
        assert!(is_ps3_disc_category("DG"));
        assert!(!is_ps3_disc_category("HG")); // homebrew
        assert!(!is_ps3_disc_category("AP")); // app
        assert!(!is_ps3_disc_category(""));
    }

    #[test]
    fn parse_basic_cnf() {
        let text = "BOOT2=cdrom0:\\SLUS_213.37;1\nVMODE=NTSC\nVER=1.00\n";
        let entries = parse_system_cnf(text);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0], SystemCnfEntry {
            key: "BOOT2".into(),
            value: "cdrom0:\\SLUS_213.37;1".into(),
        });
        assert_eq!(entries[1].key, "VMODE");
        assert_eq!(entries[1].value, "NTSC");
        assert_eq!(entries[2].value, "1.00");
    }

    #[test]
    fn parse_trims_whitespace() {
        let text = "  BOOT  =  cdrom0:\\PSX.ELF;1  \n";
        let entries = parse_system_cnf(text);
        assert_eq!(entries[0].key, "BOOT");
        assert_eq!(entries[0].value, "cdrom0:\\PSX.ELF;1");
    }

    #[test]
    fn parse_ignores_lines_without_equals() {
        let text = "BOOT2=x\n# comment\nVMODE=PAL\n";
        let entries = parse_system_cnf(text);
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn parse_empty_value_is_retained() {
        let text = "BOOT2=\n";
        let entries = parse_system_cnf(text);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].value, "");
    }

    #[test]
    fn classify_boot2_as_ps2() {
        let entries = vec![SystemCnfEntry {
            key: "BOOT2".into(),
            value: "cdrom0:".into(),
        }];
        assert_eq!(classify_system_cnf(&entries), SystemCnfDiscType::Ps2);
    }

    #[test]
    fn classify_boot_as_ps1() {
        let entries = vec![SystemCnfEntry {
            key: "BOOT".into(),
            value: "cdrom0:".into(),
        }];
        assert_eq!(classify_system_cnf(&entries), SystemCnfDiscType::Ps1);
    }

    #[test]
    fn classify_unknown_without_boot_keys() {
        let entries = vec![
            SystemCnfEntry { key: "VMODE".into(), value: "NTSC".into() },
            SystemCnfEntry { key: "VER".into(), value: "1.00".into() },
        ];
        assert_eq!(classify_system_cnf(&entries), SystemCnfDiscType::Unknown);
    }

    #[test]
    fn classify_text_end_to_end() {
        assert_eq!(
            classify_text("BOOT2=cdrom0:\\SLUS.123;1\n"),
            SystemCnfDiscType::Ps2
        );
        assert_eq!(
            classify_text("BOOT=cdrom0:\\SCES.456;1\n"),
            SystemCnfDiscType::Ps1
        );
        assert_eq!(classify_text(""), SystemCnfDiscType::Unknown);
    }
}
