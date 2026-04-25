//! `rpcs3-np-countries` — Rust port of `rpcs3/Emu/NP/rpcn_countries.cpp`.
//!
//! PSN-valid country list used by RPCN (the open replacement PSN backend
//! RPCS3 ships). The order + alpha-2 ISO codes are part of RPCN's wire
//! contract — any drift would break account creation against existing
//! RPCN servers.
//!
//! Frozen:
//!
//! - 72-entry `COUNTRIES` table (cpp:6..80) with exact name + code pairs.
//! - Japan appears at index 0 (the PSN default / "original" region).
//! - United States at index 1, then alphabetic (Argentina, Australia, …).

/// A single row of the RPCN country table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CountryCode {
    pub name: &'static str,
    pub code: &'static str,
}

/// The exact 72-entry ordering from `rpcn_countries.cpp:6..80`.
/// **Do not sort — wire contract depends on index parity.**
pub const COUNTRIES: &[CountryCode] = &[
    CountryCode { name: "Japan", code: "jp" },
    CountryCode { name: "United States", code: "us" },
    CountryCode { name: "Argentina", code: "ar" },
    CountryCode { name: "Australia", code: "au" },
    CountryCode { name: "Austria", code: "at" },
    CountryCode { name: "Bahrain", code: "bh" },
    CountryCode { name: "Belgium", code: "be" },
    CountryCode { name: "Bolivia", code: "bo" },
    CountryCode { name: "Brazil", code: "br" },
    CountryCode { name: "Bulgaria", code: "bg" },
    CountryCode { name: "Canada", code: "ca" },
    CountryCode { name: "Chile", code: "cl" },
    CountryCode { name: "China", code: "cn" },
    CountryCode { name: "Colombia", code: "co" },
    CountryCode { name: "Costa Rica", code: "cr" },
    CountryCode { name: "Croatia", code: "hr" },
    CountryCode { name: "Cyprus", code: "cy" },
    CountryCode { name: "Czech Republic", code: "cz" },
    CountryCode { name: "Denmark", code: "dk" },
    CountryCode { name: "Ecuador", code: "ec" },
    CountryCode { name: "El Salvador", code: "sv" },
    CountryCode { name: "Finland", code: "fi" },
    CountryCode { name: "France", code: "fr" },
    CountryCode { name: "Germany", code: "de" },
    CountryCode { name: "Greece", code: "gr" },
    CountryCode { name: "Guatemala", code: "gt" },
    CountryCode { name: "Honduras", code: "hn" },
    CountryCode { name: "Hong Kong", code: "hk" },
    CountryCode { name: "Hungary", code: "hu" },
    CountryCode { name: "Iceland", code: "is" },
    CountryCode { name: "India", code: "in" },
    CountryCode { name: "Indonesia", code: "id" },
    CountryCode { name: "Ireland", code: "ie" },
    CountryCode { name: "Israel", code: "il" },
    CountryCode { name: "Italy", code: "it" },
    CountryCode { name: "Korea", code: "kr" },
    CountryCode { name: "Kuwait", code: "kw" },
    CountryCode { name: "Lebanon", code: "lb" },
    CountryCode { name: "Luxembourg", code: "lu" },
    CountryCode { name: "Malaysia", code: "my" },
    CountryCode { name: "Malta", code: "mt" },
    CountryCode { name: "Mexico", code: "mx" },
    CountryCode { name: "Netherlands", code: "nl" },
    CountryCode { name: "New Zealand", code: "nz" },
    CountryCode { name: "Nicaragua", code: "ni" },
    CountryCode { name: "Norway", code: "no" },
    CountryCode { name: "Oman", code: "om" },
    CountryCode { name: "Panama", code: "pa" },
    CountryCode { name: "Paraguay", code: "py" },
    CountryCode { name: "Peru", code: "pe" },
    CountryCode { name: "Philippines", code: "ph" },
    CountryCode { name: "Poland", code: "pl" },
    CountryCode { name: "Portugal", code: "pt" },
    CountryCode { name: "Qatar", code: "qa" },
    CountryCode { name: "Romania", code: "ro" },
    CountryCode { name: "Russia", code: "ru" },
    CountryCode { name: "Saudi Arabia", code: "sa" },
    CountryCode { name: "Serbia", code: "rs" },
    CountryCode { name: "Singapore", code: "sg" },
    CountryCode { name: "Slovakia", code: "sk" },
    CountryCode { name: "South Africa", code: "za" },
    CountryCode { name: "Spain", code: "es" },
    CountryCode { name: "Sweden", code: "se" },
    CountryCode { name: "Switzerland", code: "ch" },
    CountryCode { name: "Taiwan", code: "tw" },
    CountryCode { name: "Thailand", code: "th" },
    CountryCode { name: "Turkey", code: "tr" },
    CountryCode { name: "Ukraine", code: "ua" },
    CountryCode { name: "United Arab Emirates", code: "ae" },
    CountryCode { name: "United Kingdom", code: "gb" },
    CountryCode { name: "Uruguay", code: "uy" },
    CountryCode { name: "Vietnam", code: "vn" },
];

pub const COUNTRY_COUNT: usize = 72;

/// Look up the display name for an ISO alpha-2 country code
/// (case-sensitive, lowercase only — matches the table).
#[must_use]
pub fn name_for_code(code: &str) -> Option<&'static str> {
    COUNTRIES.iter().find(|c| c.code == code).map(|c| c.name)
}

/// Reverse lookup: display name → alpha-2 code.
#[must_use]
pub fn code_for_name(name: &str) -> Option<&'static str> {
    COUNTRIES.iter().find(|c| c.name == name).map(|c| c.code)
}

/// Returns whether `code` is one of the 72 RPCN-valid alpha-2 codes.
#[must_use]
pub fn is_valid_code(code: &str) -> bool {
    COUNTRIES.iter().any(|c| c.code == code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn country_count_exactly_72() {
        assert_eq!(COUNTRY_COUNT, 72);
        assert_eq!(COUNTRIES.len(), 72);
    }

    #[test]
    fn first_entry_is_japan() {
        assert_eq!(COUNTRIES[0].name, "Japan");
        assert_eq!(COUNTRIES[0].code, "jp");
    }

    #[test]
    fn second_entry_is_us() {
        assert_eq!(COUNTRIES[1].name, "United States");
        assert_eq!(COUNTRIES[1].code, "us");
    }

    #[test]
    fn last_entry_is_vietnam() {
        assert_eq!(COUNTRIES[71].name, "Vietnam");
        assert_eq!(COUNTRIES[71].code, "vn");
    }

    #[test]
    fn no_duplicate_codes() {
        use std::collections::HashSet;
        let mut seen: HashSet<&str> = HashSet::new();
        for c in COUNTRIES {
            assert!(seen.insert(c.code), "duplicate code {}", c.code);
        }
    }

    #[test]
    fn all_codes_are_lowercase_alpha2() {
        for c in COUNTRIES {
            assert_eq!(c.code.len(), 2, "code {} not 2 chars", c.code);
            assert!(
                c.code.chars().all(|ch| ch.is_ascii_lowercase()),
                "code {} not lowercase",
                c.code
            );
        }
    }

    #[test]
    fn lookup_by_code_roundtrip() {
        assert_eq!(name_for_code("jp"), Some("Japan"));
        assert_eq!(name_for_code("us"), Some("United States"));
        assert_eq!(name_for_code("br"), Some("Brazil"));
        assert_eq!(name_for_code("XX"), None);
    }

    #[test]
    fn lookup_by_name_roundtrip() {
        assert_eq!(code_for_name("Japan"), Some("jp"));
        assert_eq!(code_for_name("Brazil"), Some("br"));
        assert_eq!(code_for_name("Middle-Earth"), None);
    }

    #[test]
    fn is_valid_code_works() {
        assert!(is_valid_code("jp"));
        assert!(is_valid_code("br"));
        assert!(is_valid_code("vn"));
        assert!(!is_valid_code("xx"));
        assert!(!is_valid_code(""));
        // Case-sensitive — "JP" isn't in the table.
        assert!(!is_valid_code("JP"));
    }

    #[test]
    fn brazil_is_index_8() {
        assert_eq!(COUNTRIES[8].code, "br");
    }
}
