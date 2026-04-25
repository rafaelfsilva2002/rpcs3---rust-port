//! `rpcs3-rsx-gl-decompiler` — Rust port of
//! `rpcs3/Emu/RSX/GL/GLCommonDecompiler.cpp`.
//!
//! Maps RSX shader varying register names to GLSL location slots used
//! by the OpenGL backend. Byte-exact from cpp:6..24.

/// Varying register name → location lookup table. 16 unique locations
/// covering RSX's 4 color interpolants, fog, and 10 texcoords (cpp aliases
/// `fogc`/`fog_c` to the same slot 5).
pub const VARYING_REGISTERS: &[(&str, i32)] = &[
    ("diff_color", 1),
    ("spec_color", 2),
    ("diff_color1", 3),
    ("spec_color1", 4),
    ("fogc", 5),
    ("fog_c", 5),
    ("tc0", 6),
    ("tc1", 7),
    ("tc2", 8),
    ("tc3", 9),
    ("tc4", 10),
    ("tc5", 11),
    ("tc6", 12),
    ("tc7", 13),
    ("tc8", 14),
    ("tc9", 15),
];

/// `get_varying_register_location(name)` (cpp:26..37). Returns `None`
/// on unknown names (cpp throws).
#[must_use]
pub fn get_varying_register_location(name: &str) -> Option<i32> {
    VARYING_REGISTERS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, loc)| *loc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_size_matches_cpp() {
        assert_eq!(VARYING_REGISTERS.len(), 16);
    }

    #[test]
    fn fogc_and_fog_c_alias_same_slot() {
        // cpp:11..12 — both map to 5.
        assert_eq!(get_varying_register_location("fogc"), Some(5));
        assert_eq!(get_varying_register_location("fog_c"), Some(5));
    }

    #[test]
    fn color_slots_1_to_4() {
        assert_eq!(get_varying_register_location("diff_color"), Some(1));
        assert_eq!(get_varying_register_location("spec_color"), Some(2));
        assert_eq!(get_varying_register_location("diff_color1"), Some(3));
        assert_eq!(get_varying_register_location("spec_color1"), Some(4));
    }

    #[test]
    fn texcoord_slots_6_to_15() {
        assert_eq!(get_varying_register_location("tc0"), Some(6));
        assert_eq!(get_varying_register_location("tc5"), Some(11));
        assert_eq!(get_varying_register_location("tc9"), Some(15));
    }

    #[test]
    fn unknown_name_returns_none() {
        assert_eq!(get_varying_register_location("tc10"), None);
        assert_eq!(get_varying_register_location("not_a_register"), None);
        assert_eq!(get_varying_register_location(""), None);
    }

    #[test]
    fn case_sensitive_lookup() {
        // cpp uses byte-exact string comparison.
        assert_eq!(get_varying_register_location("DIFF_COLOR"), None);
        assert_eq!(get_varying_register_location("tc0"), Some(6));
    }
}
