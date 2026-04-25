//! `rpcs3-util-cheat-info` — Rust port of `rpcs3/Utilities/cheat_info.cpp` + `.h`.
//!
//! RPCS3's cheat format: 9 type discriminants (`unsigned_8..signed_64`
//! plus `float_32`) + a 5-field record serialized with `@@@` separator.
//!
//! Frozen:
//!
//! - `CheatType` discriminants `0..=8` with `Max=9` sentinel.
//! - Pretty-print strings "Unsigned 8 bits" / "Float 32 bits" etc.
//! - `from_str`/`to_str` with `@@@` separator (5 parts: game,
//!   description, type(u8), offset(u32), red_script).
//! - `to_str` ends with a trailing `@@@` (cpp:52 — look closely at the
//!   concat: `... + red_script + "@@@"`).
//! - `from_str` validates: exactly 5 fields, type in `0..cheat_type_max`
//!   (0..=8).

/// Cheat type (`cheat_info.h:7..19`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheatType {
    Unsigned8 = 0,
    Unsigned16 = 1,
    Unsigned32 = 2,
    Unsigned64 = 3,
    Signed8 = 4,
    Signed16 = 5,
    Signed32 = 6,
    Signed64 = 7,
    Float32 = 8,
    Max = 9,
}

impl CheatType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unsigned8 => "Unsigned 8 bits",
            Self::Unsigned16 => "Unsigned 16 bits",
            Self::Unsigned32 => "Unsigned 32 bits",
            Self::Unsigned64 => "Unsigned 64 bits",
            Self::Signed8 => "Signed 8 bits",
            Self::Signed16 => "Signed 16 bits",
            Self::Signed32 => "Signed 32 bits",
            Self::Signed64 => "Signed 64 bits",
            Self::Float32 => "Float 32 bits",
            Self::Max => "",
        }
    }

    /// Parse from raw `u8`. Values outside `0..=8` yield `None`.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Unsigned8),
            1 => Some(Self::Unsigned16),
            2 => Some(Self::Unsigned32),
            3 => Some(Self::Unsigned64),
            4 => Some(Self::Signed8),
            5 => Some(Self::Signed16),
            6 => Some(Self::Signed32),
            7 => Some(Self::Signed64),
            8 => Some(Self::Float32),
            _ => None,
        }
    }
}

/// cpp `cheat_type_max = static_cast<u8>(cheat_type::max) = 9`.
pub const CHEAT_TYPE_MAX: u8 = 9;
pub const SEPARATOR: &str = "@@@";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheatInfo {
    pub game: String,
    pub description: String,
    pub cheat_type: CheatType,
    pub offset: u32,
    pub red_script: String,
}

impl Default for CheatInfo {
    fn default() -> Self {
        Self {
            game: String::new(),
            description: String::new(),
            cheat_type: CheatType::Max,
            offset: 0,
            red_script: String::new(),
        }
    }
}

impl CheatInfo {
    /// `to_str()` (cpp:50..54). Note the trailing `@@@` — cpp explicitly
    /// ends with it.
    #[must_use]
    pub fn to_str(&self) -> String {
        format!(
            "{game}@@@{desc}@@@{ty}@@@{off}@@@{red}@@@",
            game = self.game,
            desc = self.description,
            ty = self.cheat_type as u8,
            off = self.offset,
            red = self.red_script,
        )
    }

    /// `from_str(cheat_line)` (cpp:30..48). Returns a populated struct
    /// or `None` on invalid input (cpp returns bool + logs).
    ///
    /// cpp uses `fmt::split(... {"@@@"}, false)` which keeps empty
    /// segments, so a trailing `@@@` yields 6 parts but cpp checks
    /// `.size() != 5`, meaning trailing `@@@` is rejected. We match.
    #[must_use]
    pub fn from_str(line: &str) -> Option<Self> {
        let parts: Vec<&str> = line.split("@@@").collect();
        if parts.len() != 5 {
            return None;
        }
        let type_raw: i64 = parts[2].parse().ok()?;
        if !(0..=(CHEAT_TYPE_MAX as i64 - 1)).contains(&type_raw) {
            return None;
        }
        let cheat_type = CheatType::from_u8(type_raw as u8)?;
        let offset: u32 = parts[3].parse().ok()?;
        Some(Self {
            game: parts[0].to_string(),
            description: parts[1].to_string(),
            cheat_type,
            offset,
            red_script: parts[4].to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cheat_type_discriminants() {
        assert_eq!(CheatType::Unsigned8 as u8, 0);
        assert_eq!(CheatType::Float32 as u8, 8);
        assert_eq!(CheatType::Max as u8, CHEAT_TYPE_MAX);
    }

    #[test]
    fn as_str_strings() {
        assert_eq!(CheatType::Unsigned8.as_str(), "Unsigned 8 bits");
        assert_eq!(CheatType::Signed64.as_str(), "Signed 64 bits");
        assert_eq!(CheatType::Float32.as_str(), "Float 32 bits");
        assert_eq!(CheatType::Max.as_str(), "");
    }

    #[test]
    fn from_u8_range() {
        for i in 0..=8 {
            assert!(CheatType::from_u8(i).is_some(), "i={i}");
        }
        assert_eq!(CheatType::from_u8(9), None);
        assert_eq!(CheatType::from_u8(255), None);
    }

    #[test]
    fn to_str_ends_with_trailing_separator() {
        let c = CheatInfo {
            game: "BLUS12345".into(),
            description: "Infinite HP".into(),
            cheat_type: CheatType::Unsigned32,
            offset: 0x12345678,
            red_script: "script".into(),
        };
        assert_eq!(c.to_str(), "BLUS12345@@@Infinite HP@@@2@@@305419896@@@script@@@");
    }

    #[test]
    fn from_str_rejects_bad_arity() {
        // Trailing @@@ yields 6 parts (cpp explicitly rejects this).
        assert!(CheatInfo::from_str("a@@@b@@@0@@@1@@@c@@@").is_none());
        // Too few.
        assert!(CheatInfo::from_str("a@@@b").is_none());
        // Empty.
        assert!(CheatInfo::from_str("").is_none());
    }

    #[test]
    fn from_str_happy_path() {
        let c = CheatInfo::from_str("game@@@desc@@@3@@@42@@@script").unwrap();
        assert_eq!(c.game, "game");
        assert_eq!(c.description, "desc");
        assert_eq!(c.cheat_type, CheatType::Unsigned64);
        assert_eq!(c.offset, 42);
        assert_eq!(c.red_script, "script");
    }

    #[test]
    fn from_str_type_out_of_range_rejected() {
        assert!(CheatInfo::from_str("a@@@b@@@9@@@1@@@c").is_none());
        assert!(CheatInfo::from_str("a@@@b@@@-1@@@1@@@c").is_none());
    }

    #[test]
    fn from_str_bad_offset_rejected() {
        assert!(CheatInfo::from_str("a@@@b@@@0@@@notanumber@@@c").is_none());
    }

    #[test]
    fn from_str_empty_description_allowed() {
        let c = CheatInfo::from_str("game@@@@@@0@@@10@@@script").unwrap();
        assert_eq!(c.description, "");
    }

    #[test]
    fn round_trip_to_then_from_no_trailing_matches_when_stripped() {
        let c = CheatInfo {
            game: "G".into(),
            description: "D".into(),
            cheat_type: CheatType::Signed16,
            offset: 7,
            red_script: "S".into(),
        };
        let s = c.to_str();
        // to_str has trailing "@@@" which breaks from_str arity check
        // (cpp intentional). Strip it for round-trip.
        let stripped = s.strip_suffix("@@@").unwrap();
        let parsed = CheatInfo::from_str(stripped).unwrap();
        assert_eq!(parsed, c);
    }

    #[test]
    fn default_is_max_type_and_empty_strings() {
        let c = CheatInfo::default();
        assert_eq!(c.cheat_type, CheatType::Max);
        assert_eq!(c.offset, 0);
        assert!(c.game.is_empty());
    }
}
