//! `rpcs3-system-config-random` — Rust port of
//! `rpcs3/Emu/system_config.cpp` helpers.
//!
//! Two small helpers that generate defaults on first boot:
//!
//! - `get_random_system_name()` — returns `"RPCS3-{N}"` where N is a
//!   pseudo-random integer in `[100, 999]` (cpp:11..15 uses
//!   `std::srand(time(nullptr))` + `rand() % 899 + 100`).
//! - `get_random_psid()` — returns a 128-bit random bundle (cpp:17..30
//!   uses `std::mt19937` seeded 4x by `std::random_device` to fill
//!   each 32-bit quarter).
//!
//! We mirror the **shape** of the generators without re-using libcpp's
//! PRNG. Each helper accepts a seed/entropy source so callers can opt
//! for determinism in tests or production-quality entropy in frontends.

/// Format a system name from a random index. Takes the `rand_value`
/// (from whatever PRNG the caller picked) and applies the cpp:14 shape:
/// `"RPCS3-" + to_string(100 + rand() % 899)`.
#[must_use]
pub fn format_system_name_from_raw(rand_value: u32) -> String {
    format!("RPCS3-{}", 100 + (rand_value % 899))
}

/// Produce a 128-bit PSID from four 32-bit random words (cpp:24..27).
/// The cpp accumulates via `+=`, which is equivalent to OR-in because
/// the shifts place each word into non-overlapping 32-bit slots. We OR
/// for clarity + slightly faster code.
#[must_use]
pub fn compose_psid(q0: u32, q1: u32, q2: u32, q3: u32) -> u128 {
    u128::from(q0)
        | (u128::from(q1) << 32)
        | (u128::from(q2) << 64)
        | (u128::from(q3) << 96)
}

/// Byte layout of the formatted system name. Lower bound is 8 chars
/// (`"RPCS3-NNN"` minimum 100 → `"RPCS3-100"` = 9 chars, the + sign is
/// not emitted). Upper bound 9 chars.
#[must_use]
pub fn system_name_in_range(name: &str) -> bool {
    if !name.starts_with("RPCS3-") {
        return false;
    }
    let rest = &name["RPCS3-".len()..];
    if let Ok(n) = rest.parse::<u32>() {
        n >= 100 && n <= 998
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_system_name_basic_shape() {
        assert_eq!(format_system_name_from_raw(0), "RPCS3-100");
        assert_eq!(format_system_name_from_raw(898), "RPCS3-998");
        // Wrap: 899 % 899 = 0 → 100.
        assert_eq!(format_system_name_from_raw(899), "RPCS3-100");
        // 1798 % 899 = 0, not 1797 (which gives 898).
        assert_eq!(format_system_name_from_raw(1798), "RPCS3-100");
        assert_eq!(format_system_name_from_raw(1797), "RPCS3-998");
        assert_eq!(format_system_name_from_raw(1), "RPCS3-101");
    }

    #[test]
    fn format_system_name_always_starts_with_rpcs3() {
        for v in [0u32, 1, 100, 1000, 0xFFFF_FFFF] {
            assert!(format_system_name_from_raw(v).starts_with("RPCS3-"));
        }
    }

    #[test]
    fn system_name_in_range_accepts_generated() {
        for v in [0u32, 10, 100, 500, 898, 12345] {
            let name = format_system_name_from_raw(v);
            assert!(system_name_in_range(&name), "{name}");
        }
    }

    #[test]
    fn system_name_in_range_rejects_invalid() {
        assert!(!system_name_in_range("RPCS3-99"));
        assert!(!system_name_in_range("RPCS3-999"));
        assert!(!system_name_in_range("RPCS3-1000"));
        assert!(!system_name_in_range("SOMETHING-500"));
        assert!(!system_name_in_range(""));
    }

    #[test]
    fn compose_psid_places_words_in_non_overlapping_quarters() {
        let psid = compose_psid(0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444);
        assert_eq!(psid & 0xFFFF_FFFF, 0x1111_1111);
        assert_eq!((psid >> 32) & 0xFFFF_FFFF, 0x2222_2222);
        assert_eq!((psid >> 64) & 0xFFFF_FFFF, 0x3333_3333);
        assert_eq!((psid >> 96) & 0xFFFF_FFFF, 0x4444_4444);
    }

    #[test]
    fn compose_psid_zero_all_quarters() {
        assert_eq!(compose_psid(0, 0, 0, 0), 0);
    }

    #[test]
    fn compose_psid_single_quarter_set() {
        assert_eq!(compose_psid(0xFF, 0, 0, 0), 0xFF);
        assert_eq!(compose_psid(0, 0xFF, 0, 0), 0xFF << 32);
        assert_eq!(compose_psid(0, 0, 0xFF, 0), 0xFFu128 << 64);
        assert_eq!(compose_psid(0, 0, 0, 0xFF), 0xFFu128 << 96);
    }

    #[test]
    fn compose_psid_all_ones_yields_full_u128() {
        let psid = compose_psid(!0u32, !0, !0, !0);
        assert_eq!(psid, u128::MAX);
    }
}
