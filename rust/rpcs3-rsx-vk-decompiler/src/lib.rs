//! `rpcs3-rsx-vk-decompiler` — Rust port of
//! `rpcs3/Emu/RSX/VK/VKCommonDecompiler.cpp`.
//!
//! Vulkan backend's shader-varying table (different ordering from the
//! OpenGL backend — tc0..=tc9 occupy slots 0..=9, colors occupy 10..=13,
//! fog at 14, plus a custom `usr` slot at 15). Also a tiny helper that
//! extracts a trailing digit pair from a texture sampler name.
//!
//! Frozen:
//!
//! - 17 entries in the varying table (cpp:6..25). `fog_c` / `fogc` alias
//!   the same slot 14.
//! - `get_texture_index(name)`: reads the last up-to-2 digits of `name`
//!   as an integer. Empty digit run → invalid (cpp throws, we return
//!   None). Length < 2 → invalid (cpp:43..45).

/// Vulkan varying register name → location table (cpp:6..25).
pub const VARYING_REGISTERS: &[(&str, i32)] = &[
    ("tc0", 0),
    ("tc1", 1),
    ("tc2", 2),
    ("tc3", 3),
    ("tc4", 4),
    ("tc5", 5),
    ("tc6", 6),
    ("tc7", 7),
    ("tc8", 8),
    ("tc9", 9),
    ("diff_color", 10),
    ("diff_color1", 11),
    ("spec_color", 12),
    ("spec_color1", 13),
    ("fog_c", 14),
    ("fogc", 14),
    ("usr", 15),
];

/// `get_varying_register_location(name)` (cpp:27..38). Returns `None` on
/// unknown names.
#[must_use]
pub fn get_varying_register_location(name: &str) -> Option<i32> {
    VARYING_REGISTERS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, loc)| *loc)
}

/// `get_texture_index(name)` (cpp:40..65). Collects trailing digits
/// (up to 2) from `name` and parses them as an integer. Returns `None`
/// on malformed inputs.
#[must_use]
pub fn get_texture_index(name: &str) -> Option<i32> {
    if name.len() < 2 {
        return None;
    }

    const MAX_INDEX_LENGTH: usize = 2;
    let name_bytes = name.as_bytes();
    let start = name.len().saturating_sub(MAX_INDEX_LENGTH);

    let mut digits = String::new();
    for &b in &name_bytes[start..] {
        if b.is_ascii_digit() {
            digits.push(b as char);
        }
    }

    if digits.is_empty() {
        return None;
    }

    digits.parse::<i32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_has_17_entries() {
        assert_eq!(VARYING_REGISTERS.len(), 17);
    }

    #[test]
    fn tc_occupies_slots_0_to_9() {
        for i in 0..=9 {
            let name = format!("tc{i}");
            assert_eq!(get_varying_register_location(&name), Some(i as i32));
        }
    }

    #[test]
    fn color_slots_10_to_13() {
        assert_eq!(get_varying_register_location("diff_color"), Some(10));
        assert_eq!(get_varying_register_location("diff_color1"), Some(11));
        assert_eq!(get_varying_register_location("spec_color"), Some(12));
        assert_eq!(get_varying_register_location("spec_color1"), Some(13));
    }

    #[test]
    fn fog_aliases_collapse_to_14() {
        assert_eq!(get_varying_register_location("fog_c"), Some(14));
        assert_eq!(get_varying_register_location("fogc"), Some(14));
    }

    #[test]
    fn usr_is_slot_15() {
        assert_eq!(get_varying_register_location("usr"), Some(15));
    }

    #[test]
    fn unknown_name_returns_none() {
        assert_eq!(get_varying_register_location("tc10"), None);
        assert_eq!(get_varying_register_location(""), None);
    }

    #[test]
    fn texture_index_extraction_standard() {
        assert_eq!(get_texture_index("tex0"), Some(0));
        assert_eq!(get_texture_index("tex5"), Some(5));
        assert_eq!(get_texture_index("tex12"), Some(12));
        assert_eq!(get_texture_index("tex99"), Some(99));
    }

    #[test]
    fn texture_index_too_short_name() {
        // cpp:43..45 throws on name.length() < 2.
        assert_eq!(get_texture_index("a"), None);
        assert_eq!(get_texture_index(""), None);
    }

    #[test]
    fn texture_index_no_trailing_digits() {
        // cpp:59..61 throws on empty digits.
        assert_eq!(get_texture_index("sampler_foo"), None);
    }

    #[test]
    fn texture_index_only_last_two_chars_examined() {
        // "texture_100" — we only look at last 2 chars ("00") → 0.
        assert_eq!(get_texture_index("texture_100"), Some(0));
        // "tex_9" (last 2 = "_9") → 9.
        assert_eq!(get_texture_index("tex_9"), Some(9));
    }
}
