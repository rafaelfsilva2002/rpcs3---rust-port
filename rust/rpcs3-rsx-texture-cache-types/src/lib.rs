//! `rpcs3-rsx-texture-cache-types` — Rust port of
//! `rpcs3/Emu/RSX/Common/texture_cache_types.cpp` + `.h`.
//!
//! Freezes the bit-flag table that `invalidation_cause::flag_bits_from_cause`
//! uses: an 8-entry lookup from `InvalidationCause` enum to `u32` flag
//! bits. One edge case preserved: `superseded_by_fbo` under
//! `strict_texture_flushing=true` additionally clears the
//! `cause_skips_flush` bit (cpp:31..35).
//!
//! Frozen:
//!
//! - `InvalidationCause` enum (9 variants including `invalid`).
//! - Flag bits (cpp header:53..60): 8 single-bit flags.
//! - `MemoryReadFlags` (cpp header:29..33): `flush_always=0`, `flush_once=1`.
//! - `cause_flag_bits(cause, strict_texture_flushing)` — full decision
//!   table matching cpp:7..36.

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidationCause {
    Invalid = 0,
    Read = 1,
    DeferredRead = 2,
    Write = 3,
    DeferredWrite = 4,
    /// Fault range is being unmapped.
    Unmap = 5,
    /// We're about to reprotect the fault range.
    Reprotect = 6,
    /// Used by `texture_cache::locked_memory_region`.
    SupersededByFbo = 7,
    /// Same as `SupersededByFbo` but without locking / preserving page flags.
    CommittedAsFbo = 8,
}

// Flag bits (cpp header:53..60).
pub const CAUSE_IS_VALID: u32 = 1 << 0;
pub const CAUSE_IS_READ: u32 = 1 << 1;
pub const CAUSE_IS_WRITE: u32 = 1 << 2;
pub const CAUSE_IS_DEFERRED: u32 = 1 << 3;
pub const CAUSE_SKIPS_FBOS: u32 = 1 << 4;
pub const CAUSE_SKIPS_FLUSH: u32 = 1 << 5;
pub const CAUSE_KEEPS_FAULT_RANGE_PROTECTION: u32 = 1 << 6;
pub const CAUSE_USES_STRICT_DATA_BOUNDS: u32 = 1 << 7;

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryReadFlags {
    FlushAlways = 0,
    FlushOnce = 1,
}

/// Compute the `m_flag_bits` value from an invalidation cause
/// (cpp:7..36). Returns `0` (all flags cleared) for `Invalid`.
#[must_use]
pub const fn cause_flag_bits(cause: InvalidationCause, strict_texture_flushing: bool) -> u32 {
    let base: u32 = match cause {
        InvalidationCause::Invalid => 0,
        InvalidationCause::Read => CAUSE_IS_READ,
        InvalidationCause::DeferredRead => CAUSE_IS_READ | CAUSE_IS_DEFERRED,
        InvalidationCause::Write => CAUSE_IS_WRITE,
        InvalidationCause::DeferredWrite => CAUSE_IS_WRITE | CAUSE_IS_DEFERRED,
        InvalidationCause::Unmap => CAUSE_KEEPS_FAULT_RANGE_PROTECTION | CAUSE_SKIPS_FLUSH,
        InvalidationCause::Reprotect => CAUSE_KEEPS_FAULT_RANGE_PROTECTION,
        InvalidationCause::SupersededByFbo => {
            CAUSE_KEEPS_FAULT_RANGE_PROTECTION | CAUSE_SKIPS_FBOS | CAUSE_SKIPS_FLUSH
        }
        InvalidationCause::CommittedAsFbo => CAUSE_SKIPS_FBOS,
    };

    if matches!(cause, InvalidationCause::Invalid) {
        return 0;
    }

    let mut bits = base | CAUSE_IS_VALID;

    // cpp:31..35 quirk — strict_texture_flushing + superseded_by_fbo
    // strips CAUSE_SKIPS_FLUSH.
    if matches!(cause, InvalidationCause::SupersededByFbo) && strict_texture_flushing {
        bits &= !CAUSE_SKIPS_FLUSH;
    }

    bits
}

/// Whether the given flag bit set indicates a valid cause.
#[must_use]
pub const fn is_valid(flag_bits: u32) -> bool {
    (flag_bits & CAUSE_IS_VALID) != 0
}

#[must_use]
pub const fn is_read(flag_bits: u32) -> bool {
    (flag_bits & CAUSE_IS_READ) != 0
}

#[must_use]
pub const fn is_write(flag_bits: u32) -> bool {
    (flag_bits & CAUSE_IS_WRITE) != 0
}

#[must_use]
pub const fn is_deferred(flag_bits: u32) -> bool {
    (flag_bits & CAUSE_IS_DEFERRED) != 0
}

/// Downgrade a deferred cause to its non-deferred counterpart
/// (cpp:107..109).
#[must_use]
pub const fn undefer(flag_bits: u32) -> u32 {
    flag_bits & !CAUSE_IS_DEFERRED
}

/// Promote a non-deferred cause to deferred (cpp:113..115).
#[must_use]
pub const fn defer(flag_bits: u32) -> u32 {
    flag_bits | CAUSE_IS_DEFERRED
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalidation_cause_discriminants() {
        assert_eq!(InvalidationCause::Invalid as u32, 0);
        assert_eq!(InvalidationCause::Read as u32, 1);
        assert_eq!(InvalidationCause::DeferredRead as u32, 2);
        assert_eq!(InvalidationCause::Write as u32, 3);
        assert_eq!(InvalidationCause::DeferredWrite as u32, 4);
        assert_eq!(InvalidationCause::Unmap as u32, 5);
        assert_eq!(InvalidationCause::Reprotect as u32, 6);
        assert_eq!(InvalidationCause::SupersededByFbo as u32, 7);
        assert_eq!(InvalidationCause::CommittedAsFbo as u32, 8);
    }

    #[test]
    fn flag_bit_values_are_powers_of_two() {
        for f in [
            CAUSE_IS_VALID, CAUSE_IS_READ, CAUSE_IS_WRITE, CAUSE_IS_DEFERRED,
            CAUSE_SKIPS_FBOS, CAUSE_SKIPS_FLUSH, CAUSE_KEEPS_FAULT_RANGE_PROTECTION,
            CAUSE_USES_STRICT_DATA_BOUNDS,
        ] {
            assert!(f > 0 && (f & (f - 1)) == 0, "flag {f:#x} not power-of-two");
        }
    }

    #[test]
    fn memory_read_flags_values() {
        assert_eq!(MemoryReadFlags::FlushAlways as u32, 0);
        assert_eq!(MemoryReadFlags::FlushOnce as u32, 1);
    }

    #[test]
    fn invalid_cause_produces_zero() {
        assert_eq!(cause_flag_bits(InvalidationCause::Invalid, false), 0);
        assert_eq!(cause_flag_bits(InvalidationCause::Invalid, true), 0);
    }

    #[test]
    fn read_write_basic_flags() {
        let b = cause_flag_bits(InvalidationCause::Read, false);
        assert!(is_valid(b));
        assert!(is_read(b));
        assert!(!is_write(b));
        assert!(!is_deferred(b));

        let b = cause_flag_bits(InvalidationCause::Write, false);
        assert!(is_valid(b));
        assert!(!is_read(b));
        assert!(is_write(b));
        assert!(!is_deferred(b));
    }

    #[test]
    fn deferred_variants_set_deferred_bit() {
        let b = cause_flag_bits(InvalidationCause::DeferredRead, false);
        assert!(is_deferred(b));
        assert!(is_read(b));

        let b = cause_flag_bits(InvalidationCause::DeferredWrite, false);
        assert!(is_deferred(b));
        assert!(is_write(b));
    }

    #[test]
    fn unmap_has_fault_protect_and_skips_flush() {
        let b = cause_flag_bits(InvalidationCause::Unmap, false);
        assert!(b & CAUSE_KEEPS_FAULT_RANGE_PROTECTION != 0);
        assert!(b & CAUSE_SKIPS_FLUSH != 0);
        assert!(!is_read(b));
        assert!(!is_write(b));
    }

    #[test]
    fn superseded_by_fbo_normal_has_skips_flush() {
        let b = cause_flag_bits(InvalidationCause::SupersededByFbo, false);
        assert!(b & CAUSE_SKIPS_FLUSH != 0);
        assert!(b & CAUSE_SKIPS_FBOS != 0);
        assert!(b & CAUSE_KEEPS_FAULT_RANGE_PROTECTION != 0);
    }

    #[test]
    fn superseded_by_fbo_strict_clears_skips_flush() {
        // cpp:31..35 — strict_texture_flushing=true clears skips_flush.
        let b = cause_flag_bits(InvalidationCause::SupersededByFbo, true);
        assert_eq!(b & CAUSE_SKIPS_FLUSH, 0);
        // But keeps the other two flags.
        assert!(b & CAUSE_SKIPS_FBOS != 0);
        assert!(b & CAUSE_KEEPS_FAULT_RANGE_PROTECTION != 0);
    }

    #[test]
    fn committed_as_fbo_only_has_skips_fbos() {
        let b = cause_flag_bits(InvalidationCause::CommittedAsFbo, false);
        assert!(b & CAUSE_SKIPS_FBOS != 0);
        // It must NOT keep fault range protection.
        assert_eq!(b & CAUSE_KEEPS_FAULT_RANGE_PROTECTION, 0);
        // strict_flushing shouldn't touch committed_as_fbo.
        let b_strict = cause_flag_bits(InvalidationCause::CommittedAsFbo, true);
        assert_eq!(b, b_strict);
    }

    #[test]
    fn defer_undefer_round_trip() {
        let b = cause_flag_bits(InvalidationCause::Read, false);
        let d = defer(b);
        assert!(is_deferred(d));
        let u = undefer(d);
        assert!(!is_deferred(u));
        assert_eq!(u, b);
    }

    #[test]
    fn all_non_invalid_causes_set_valid_bit() {
        for c in [
            InvalidationCause::Read, InvalidationCause::DeferredRead,
            InvalidationCause::Write, InvalidationCause::DeferredWrite,
            InvalidationCause::Unmap, InvalidationCause::Reprotect,
            InvalidationCause::SupersededByFbo, InvalidationCause::CommittedAsFbo,
        ] {
            let b = cause_flag_bits(c, false);
            assert!(is_valid(b), "cause {c:?} missing valid bit");
        }
    }
}
