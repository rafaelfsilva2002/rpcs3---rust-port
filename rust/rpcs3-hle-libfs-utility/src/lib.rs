//! `rpcs3-hle-libfs-utility` — PS3 filesystem utility init HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/libfs_utility_init.cpp` — the firmware
//! module that a game calls when preparing HDD / BD partitions before
//! touching `cellFs`.  Every entry point is registered with
//! `REG_FNID(fs_utility_init, 0x________, …)` — the function IDs are
//! the PS3-visible names (the firmware strips C identifiers before
//! shipping).  In RPCS3 only one of them has any observable behaviour:
//! `0x6B5896B0` writes `*dest = 2` (number of mountable partitions).
//!
//! ## Registered FNIDs
//!
//! | FNID        | Port wrapper             | Observable behaviour       |
//! |-------------|--------------------------|----------------------------|
//! | `0x1F3CD9F1`| [`fn_1f3cd9f1`]          | stub → `CELL_OK`           |
//! | `0x263172B8`| [`fn_263172b8`]          | stub → `CELL_OK`           |
//! | `0x4E949DA4`| [`fn_4e949da4`]          | stub → `CELL_OK`           |
//! | `0x665DF255`| [`fn_665df255`]          | stub → `CELL_OK`           |
//! | `0x6B5896B0`| [`fn_6b5896b0`]          | writes partition count `2` |
//! | `0xA9B04535`| [`fn_a9b04535`]          | stub → `CELL_OK`           |
//! | `0xE7563CE6`| [`fn_e7563ce6`]          | stub → `CELL_OK`           |
//! | `0xF691D443`| [`fn_f691d443`]          | stub → `CELL_OK`           |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes
// =====================================================================

/// `CELL_EFAULT` — returned by `fn_6b5896b0` when the destination
/// pointer is null (`if (!dest) return CELL_EFAULT;`).
pub const CELL_EFAULT: CellError = CellError(0x8001_000D);

// =====================================================================
// Constants
// =====================================================================

/// Number of mountable partitions reported by `fn_6b5896b0`.
///
/// The firmware hard-codes `*dest = 2` on success — see
/// libfs_utility_init.cpp:46.
pub const NUM_PARTITIONS: u64 = 2;

/// Table of all 8 FNIDs registered for the module, in the order they
/// appear in `REG_FNID` calls (libfs_utility_init.cpp:77-84).
pub const REGISTERED_FNIDS: [u32; 8] = [
    0x1F3CD9F1,
    0x263172B8,
    0x4E949DA4,
    0x665DF255,
    0x6B5896B0,
    0xA9B04535,
    0xE7563CE6,
    0xF691D443,
];

// =====================================================================
// Entry points
// =====================================================================

/// Stub for FNID `0x1F3CD9F1`.
#[must_use]
pub fn fn_1f3cd9f1() -> Result<(), CellError> { Ok(()) }

/// Stub for FNID `0x263172B8`.  Firmware accepts any `arg1`.
///
/// The C++ comment says "Negative numbers indicate an error / Some
/// positive numbers are deemed illegal, others (including 0) are
/// accepted as valid" — the current implementation is a pure stub, so
/// we match by returning `Ok(())` regardless.
#[must_use]
pub fn fn_263172b8(_arg1: u32) -> Result<(), CellError> { Ok(()) }

/// Stub for FNID `0x4E949DA4`.
#[must_use]
pub fn fn_4e949da4() -> Result<(), CellError> { Ok(()) }

/// Stub for FNID `0x665DF255`.
#[must_use]
pub fn fn_665df255() -> Result<(), CellError> { Ok(()) }

/// Port of FNID `0x6B5896B0` — writes the partition count to `dest`.
///
/// Model the `vm::ptr<u64>` argument with `Option<&mut u64>`: `None`
/// represents a null pointer (triggers `CELL_EFAULT`), `Some(slot)`
/// receives the partition count.
///
/// # Errors
/// * [`CELL_EFAULT`] if `dest` is null.
pub fn fn_6b5896b0(dest: Option<&mut u64>) -> Result<(), CellError> {
    let Some(slot) = dest else { return Err(CELL_EFAULT) };
    *slot = NUM_PARTITIONS;
    Ok(())
}

/// Stub for FNID `0xA9B04535`.  See `fn_263172b8` for the comment about
/// validity — the firmware accepts any `arg1` and returns `CELL_OK`.
#[must_use]
pub fn fn_a9b04535(_arg1: u32) -> Result<(), CellError> { Ok(()) }

/// Stub for FNID `0xE7563CE6`.
#[must_use]
pub fn fn_e7563ce6() -> Result<(), CellError> { Ok(()) }

/// Stub for FNID `0xF691D443`.
#[must_use]
pub fn fn_f691d443() -> Result<(), CellError> { Ok(()) }

/// Look up the observable action for a given FNID.  Returns
/// `Some(())` for stubs (caller treats as `CELL_OK`) or an error for
/// mis-matched FNIDs.  The 5th FNID (`0x6B5896B0`) has observable
/// state, so callers that need the partition-count write must call
/// [`fn_6b5896b0`] directly.
///
/// # Errors
/// Returns the unknown-FNID marker `CELL_EFAULT` if `fnid` isn't one of
/// [`REGISTERED_FNIDS`].  (The firmware's real unknown-FNID handler is
/// PRX-level and outside this module's surface; `CELL_EFAULT` is the
/// closest observable for testing.)
pub fn lookup_fnid(fnid: u32) -> Result<(), CellError> {
    if REGISTERED_FNIDS.contains(&fnid) {
        Ok(())
    } else {
        Err(CELL_EFAULT)
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- constants ---------------------------------------------------

    #[test]
    fn cell_efault_byte_exact() {
        assert_eq!(CELL_EFAULT.0, 0x8001_000D);
    }

    #[test]
    fn num_partitions_matches_firmware() {
        // libfs_utility_init.cpp:46 hard-codes `*dest = 2`.
        assert_eq!(NUM_PARTITIONS, 2);
    }

    #[test]
    fn fnid_registry_has_eight_entries() {
        assert_eq!(REGISTERED_FNIDS.len(), 8);
    }

    #[test]
    fn fnid_registry_matches_cpp_order() {
        // Order from libfs_utility_init.cpp:77-84.
        assert_eq!(REGISTERED_FNIDS, [
            0x1F3CD9F1, 0x263172B8, 0x4E949DA4, 0x665DF255,
            0x6B5896B0, 0xA9B04535, 0xE7563CE6, 0xF691D443,
        ]);
    }

    #[test]
    fn fnid_registry_has_no_duplicates() {
        let mut sorted = REGISTERED_FNIDS;
        sorted.sort_unstable();
        for pair in sorted.windows(2) {
            assert_ne!(pair[0], pair[1], "duplicate FNID {:#x}", pair[0]);
        }
    }

    // ---- stubs ------------------------------------------------------

    #[test]
    fn stub_entry_points_all_ok() {
        assert!(fn_1f3cd9f1().is_ok());
        assert!(fn_263172b8(0).is_ok());
        assert!(fn_263172b8(0x3).is_ok()); // the typical arg per C++ comment
        assert!(fn_4e949da4().is_ok());
        assert!(fn_665df255().is_ok());
        assert!(fn_a9b04535(0).is_ok());
        assert!(fn_a9b04535(0xFFFF_FFFF).is_ok());
        assert!(fn_e7563ce6().is_ok());
        assert!(fn_f691d443().is_ok());
    }

    // ---- fn_6b5896b0 (observable write) -----------------------------

    #[test]
    fn fn_6b5896b0_writes_partition_count() {
        let mut dest: u64 = 0xDEAD_BEEF;
        fn_6b5896b0(Some(&mut dest)).unwrap();
        assert_eq!(dest, 2);
    }

    #[test]
    fn fn_6b5896b0_null_pointer_returns_efault() {
        let err = fn_6b5896b0(None).unwrap_err();
        assert_eq!(err, CELL_EFAULT);
    }

    #[test]
    fn fn_6b5896b0_is_idempotent() {
        let mut dest: u64 = 0;
        fn_6b5896b0(Some(&mut dest)).unwrap();
        fn_6b5896b0(Some(&mut dest)).unwrap();
        assert_eq!(dest, 2);
    }

    #[test]
    fn fn_6b5896b0_overwrites_previous_value() {
        let mut dest: u64 = 0xFFFF_FFFF_FFFF_FFFF;
        fn_6b5896b0(Some(&mut dest)).unwrap();
        assert_eq!(dest, NUM_PARTITIONS);
    }

    // ---- lookup_fnid ------------------------------------------------

    #[test]
    fn lookup_fnid_accepts_all_registered() {
        for fnid in REGISTERED_FNIDS {
            assert!(lookup_fnid(fnid).is_ok(), "{fnid:#x} should be known");
        }
    }

    #[test]
    fn lookup_fnid_rejects_unknown() {
        assert_eq!(lookup_fnid(0x0000_0000).unwrap_err(), CELL_EFAULT);
        assert_eq!(lookup_fnid(0xDEAD_BEEF).unwrap_err(), CELL_EFAULT);
        // Close-but-not-equal: flip one bit.
        assert_eq!(lookup_fnid(0x1F3CD9F0).unwrap_err(), CELL_EFAULT);
    }

    // ---- individual FNID sanity -------------------------------------

    #[test]
    fn fnid_1f3cd9f1_byte_exact() {
        assert_eq!(REGISTERED_FNIDS[0], 0x1F3CD9F1);
    }

    #[test]
    fn fnid_263172b8_byte_exact() {
        assert_eq!(REGISTERED_FNIDS[1], 0x263172B8);
    }

    #[test]
    fn fnid_4e949da4_byte_exact() {
        assert_eq!(REGISTERED_FNIDS[2], 0x4E949DA4);
    }

    #[test]
    fn fnid_665df255_byte_exact() {
        assert_eq!(REGISTERED_FNIDS[3], 0x665DF255);
    }

    #[test]
    fn fnid_6b5896b0_byte_exact() {
        assert_eq!(REGISTERED_FNIDS[4], 0x6B5896B0);
    }

    #[test]
    fn fnid_a9b04535_byte_exact() {
        assert_eq!(REGISTERED_FNIDS[5], 0xA9B04535);
    }

    #[test]
    fn fnid_e7563ce6_byte_exact() {
        assert_eq!(REGISTERED_FNIDS[6], 0xE7563CE6);
    }

    #[test]
    fn fnid_f691d443_byte_exact() {
        assert_eq!(REGISTERED_FNIDS[7], 0xF691D443);
    }

    // ---- full smoke -------------------------------------------------

    #[test]
    fn full_libfs_utility_lifecycle_smoke() {
        // Game calls every entry point once during boot setup.
        fn_1f3cd9f1().unwrap();
        fn_263172b8(0x3).unwrap();
        fn_4e949da4().unwrap();
        fn_665df255().unwrap();

        let mut partitions: u64 = 0;
        fn_6b5896b0(Some(&mut partitions)).unwrap();
        assert_eq!(partitions, 2, "firmware reports 2 partitions");

        fn_a9b04535(0x3).unwrap();
        fn_e7563ce6().unwrap();
        fn_f691d443().unwrap();

        // Unknown FNID is rejected.
        assert_eq!(lookup_fnid(0xDEAD_BEEF).unwrap_err(), CELL_EFAULT);

        // Null dest to the observable fn yields EFAULT.
        assert_eq!(fn_6b5896b0(None).unwrap_err(), CELL_EFAULT);
    }
}
