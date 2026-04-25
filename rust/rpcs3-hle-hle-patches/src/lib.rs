//! `rpcs3-hle-hle-patches` — RPCS3 per-game HLE fixups.
//!
//! Ports `rpcs3/Emu/Cell/Modules/HLE_PATCHES.cpp` (57 linhas).  The
//! module exists to ship **one** function registered under
//! `RPCS3_HLE_LIBRARY`: `WaitForSPUsToEmptySNRs` — a workaround for a
//! race condition in *Sonic the Hedgehog* where PPU → SPU signalling
//! races cause missing graphics when the SNR (Signal Notification
//! Register) gets overwritten while still non-empty.
//!
//! The logic from cpp:11-52 is:
//!
//! * `snr_mask % 4 == 0` (bits 0 and 1 both unset) → nothing to wait
//!   for, return immediately.
//! * `spu_id == umax` and SPU not found → return immediately.
//! * Otherwise spin-wait until:
//!   * if `spu_id` names a specific SPU: its SNR1/SNR2 counts are zero
//!     for the requested bits;
//!   * if `spu_id == umax`: **all** SPUs in the group report zero.
//!
//! ## Entry points covered
//!
//! | C++ function               | Rust wrapper              |
//! |----------------------------|---------------------------|
//! | `WaitForSPUsToEmptySNRs`   | [`wait_for_spus_to_empty_snrs`] |

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — only one used in this module
// =====================================================================

/// `CELL_EINVAL` — not actually emitted by the firmware here, but we
/// surface it for out-of-bounds SPU lookups in higher-level tests.
pub const CELL_EINVAL: CellError = CellError(0x8001_0002);

// =====================================================================
// Constants — byte-exact with the SPU-signal semantics
// =====================================================================

/// SPU id sentinel meaning "all SPUs in the group" (matches the `umax`
/// check in cpp:17).  On PPU `umax` is `0xFFFFFFFF`.
pub const SPU_ID_ALL: u32 = 0xFFFF_FFFF;

/// SNR mask bit 0 → SNR1.  `cpp:29 (snr_mask & 1)`.
pub const SNR_MASK_SNR1: u32 = 0b01;

/// SNR mask bit 1 → SNR2.  `cpp:35 (snr_mask & 2)`.
pub const SNR_MASK_SNR2: u32 = 0b10;

/// Mask bits that actually address a register.  Bits 0 & 1 together =
/// `0b11`; `snr_mask % 4 == 0` is the early-return path.
pub const SNR_MASK_RELEVANT: u32 = 0b11;

// =====================================================================
// SPU SNR state
// =====================================================================

/// Mirror of the fields cpp:29-38 inspects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SpuSnrState {
    pub id: u32,
    pub snr1_count: u32,
    pub snr2_count: u32,
}

impl SpuSnrState {
    #[must_use]
    pub const fn new(id: u32) -> Self { Self { id, snr1_count: 0, snr2_count: 0 } }

    /// Returns `true` if the SNRs the caller requested are currently
    /// busy (count != 0).  Mirrors the two `if` branches in cpp:29-38.
    #[must_use]
    pub fn has_busy(&self, snr_mask: u32) -> bool {
        if (snr_mask & SNR_MASK_SNR1) != 0 && self.snr1_count != 0 { return true; }
        if (snr_mask & SNR_MASK_SNR2) != 0 && self.snr2_count != 0 { return true; }
        false
    }
}

/// Fake SPU-group registry — higher layers provide real SPU threads
/// via a slice.  The port captures just enough behaviour to exercise
/// the `get_thread + idm::select` fork in cpp:41-50.
#[derive(Debug, Clone, Default)]
pub struct SpuGroup {
    pub spus: Vec<SpuSnrState>,
}

impl SpuGroup {
    #[must_use]
    pub fn new() -> Self { Self { spus: Vec::new() } }

    #[must_use]
    pub fn with_spus(spus: Vec<SpuSnrState>) -> Self { Self { spus } }

    /// Scan the group for the first SPU whose id matches.  Returns
    /// `None` when the id is absent (modelling `get_thread(spu_id)`
    /// failure).
    #[must_use]
    pub fn find(&self, spu_id: u32) -> Option<&SpuSnrState> {
        self.spus.iter().find(|s| s.id == spu_id)
    }

    /// Returns `true` if ANY SPU in the group still has busy SNRs for
    /// the requested mask.  Matches the `idm::select` path in cpp:49.
    #[must_use]
    pub fn any_busy(&self, snr_mask: u32) -> bool {
        self.spus.iter().any(|s| s.has_busy(snr_mask))
    }

    /// Decrement both SNR counters on every SPU by one (saturating).
    /// Test helper that simulates the firmware draining the registers.
    pub fn drain_one(&mut self, snr_mask: u32) {
        for s in &mut self.spus {
            if (snr_mask & SNR_MASK_SNR1) != 0 {
                s.snr1_count = s.snr1_count.saturating_sub(1);
            }
            if (snr_mask & SNR_MASK_SNR2) != 0 {
                s.snr2_count = s.snr2_count.saturating_sub(1);
            }
        }
    }
}

// =====================================================================
// Port of `WaitForSPUsToEmptySNRs` (cpp:11-52)
// =====================================================================

/// Outcome of the early-return analysis in cpp:17-20.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    /// `spu_id != umax` but the SPU isn't in the group — return
    /// immediately (matches `if ((!spu && spu_id != umax) …)`).
    SpuNotFound,
    /// `snr_mask % 4 == 0` — caller didn't request any SNR, so no work.
    MaskIsEmpty,
    /// Ready to enter the busy-wait loop.  `all_spus = true` when the
    /// caller passed `SPU_ID_ALL`.
    WouldSpin { all_spus: bool },
}

/// Port of `WaitForSPUsToEmptySNRs` — just the decision logic
/// (cpp:11-20).  The actual busy-wait spin is left to the caller
/// because it depends on real SPU thread state.
#[must_use]
pub fn wait_outcome(group: &SpuGroup, spu_id: u32, snr_mask: u32) -> WaitOutcome {
    // cpp:17 — `if ((!spu && spu_id != umax) || snr_mask % 4 == 0) return;`
    let spu_missing = spu_id != SPU_ID_ALL && group.find(spu_id).is_none();
    if spu_missing { return WaitOutcome::SpuNotFound; }
    if snr_mask % 4 == 0 { return WaitOutcome::MaskIsEmpty; }
    WaitOutcome::WouldSpin { all_spus: spu_id == SPU_ID_ALL }
}

/// Perform the bounded spin-wait (cpp:23-51) given the decision already
/// classified by [`wait_outcome`].  `max_iters` caps the loop for
/// deterministic tests (C++ spins indefinitely).  Returns `Ok(())` on
/// drain, `Err(CELL_EINVAL)` if the spin cap is exhausted.
///
/// # Errors
/// * [`CELL_EINVAL`] if the spin cap is hit before the SNRs drain.
pub fn wait_for_spus_to_empty_snrs(
    group: &mut SpuGroup,
    spu_id: u32,
    snr_mask: u32,
    max_iters: u32,
    drain_per_iter: bool,
) -> Result<WaitResult, CellError> {
    match wait_outcome(group, spu_id, snr_mask) {
        WaitOutcome::SpuNotFound   => return Ok(WaitResult::ReturnedEarly),
        WaitOutcome::MaskIsEmpty   => return Ok(WaitResult::ReturnedEarly),
        WaitOutcome::WouldSpin { .. } => {}
    }
    for _ in 0..max_iters {
        let has_busy = if spu_id == SPU_ID_ALL {
            group.any_busy(snr_mask)
        } else {
            group.find(spu_id).is_some_and(|s| s.has_busy(snr_mask))
        };
        if !has_busy {
            return Ok(WaitResult::Drained);
        }
        if drain_per_iter {
            group.drain_one(snr_mask);
        }
    }
    Err(CELL_EINVAL)
}

/// Result of a successful [`wait_for_spus_to_empty_snrs`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitResult {
    /// Pre-check returned before entering the spin loop.
    ReturnedEarly,
    /// Spin loop observed all requested SNRs empty.
    Drained,
}

// =====================================================================
// Registry
// =====================================================================

pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "WaitForSPUsToEmptySNRs",
];

/// Module name used by `REG_FUNC(RPCS3_HLE_LIBRARY, …)` (cpp:54).
pub const MODULE_NAME: &str = "RPCS3_HLE_LIBRARY";

#[must_use]
pub fn is_registered(name: &str) -> bool {
    REGISTERED_ENTRY_POINTS.contains(&name)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- constants ---------------------------------------------------

    #[test]
    fn constants_byte_exact() {
        assert_eq!(SPU_ID_ALL, 0xFFFF_FFFF);
        assert_eq!(SNR_MASK_SNR1, 0b01);
        assert_eq!(SNR_MASK_SNR2, 0b10);
        assert_eq!(SNR_MASK_RELEVANT, 0b11);
    }

    #[test]
    fn module_name_byte_exact() {
        assert_eq!(MODULE_NAME, "RPCS3_HLE_LIBRARY");
    }

    // ---- SpuSnrState::has_busy --------------------------------------

    #[test]
    fn has_busy_empty_registers() {
        let s = SpuSnrState::new(0);
        assert!(!s.has_busy(SNR_MASK_SNR1));
        assert!(!s.has_busy(SNR_MASK_SNR2));
        assert!(!s.has_busy(SNR_MASK_RELEVANT));
    }

    #[test]
    fn has_busy_snr1_only_when_requested() {
        let s = SpuSnrState { id: 0, snr1_count: 1, snr2_count: 0 };
        assert!(s.has_busy(SNR_MASK_SNR1));
        assert!(!s.has_busy(SNR_MASK_SNR2));
        assert!(s.has_busy(SNR_MASK_RELEVANT));
    }

    #[test]
    fn has_busy_snr2_only_when_requested() {
        let s = SpuSnrState { id: 0, snr1_count: 0, snr2_count: 1 };
        assert!(!s.has_busy(SNR_MASK_SNR1));
        assert!(s.has_busy(SNR_MASK_SNR2));
        assert!(s.has_busy(SNR_MASK_RELEVANT));
    }

    #[test]
    fn has_busy_both_when_requested() {
        let s = SpuSnrState { id: 0, snr1_count: 3, snr2_count: 5 };
        assert!(s.has_busy(SNR_MASK_RELEVANT));
    }

    #[test]
    fn has_busy_ignores_non_mask_bits() {
        // Bits above 1 should be ignored — matches the C++ checks that
        // only look at `snr_mask & 1` and `snr_mask & 2`.
        let s = SpuSnrState { id: 0, snr1_count: 0, snr2_count: 0 };
        assert!(!s.has_busy(0x100)); // no SNR bits set
    }

    // ---- SpuGroup ---------------------------------------------------

    #[test]
    fn group_find_returns_matching_spu() {
        let g = SpuGroup::with_spus(alloc::vec![
            SpuSnrState::new(10),
            SpuSnrState::new(20),
            SpuSnrState::new(30),
        ]);
        assert!(g.find(20).is_some());
        assert!(g.find(99).is_none());
    }

    #[test]
    fn group_any_busy_false_when_all_empty() {
        let g = SpuGroup::with_spus(alloc::vec![
            SpuSnrState::new(0),
            SpuSnrState::new(1),
        ]);
        assert!(!g.any_busy(SNR_MASK_RELEVANT));
    }

    #[test]
    fn group_any_busy_true_when_one_has_busy() {
        let g = SpuGroup::with_spus(alloc::vec![
            SpuSnrState::new(0),
            SpuSnrState { id: 1, snr1_count: 1, snr2_count: 0 },
        ]);
        assert!(g.any_busy(SNR_MASK_SNR1));
        assert!(!g.any_busy(SNR_MASK_SNR2));
    }

    #[test]
    fn group_drain_one_decrements_saturating() {
        let mut g = SpuGroup::with_spus(alloc::vec![
            SpuSnrState { id: 0, snr1_count: 2, snr2_count: 3 },
        ]);
        g.drain_one(SNR_MASK_RELEVANT);
        assert_eq!(g.spus[0].snr1_count, 1);
        assert_eq!(g.spus[0].snr2_count, 2);
        // Three more drains empty everything.
        for _ in 0..5 {
            g.drain_one(SNR_MASK_RELEVANT);
        }
        assert_eq!(g.spus[0].snr1_count, 0);
        assert_eq!(g.spus[0].snr2_count, 0);
    }

    #[test]
    fn group_drain_one_respects_mask() {
        let mut g = SpuGroup::with_spus(alloc::vec![
            SpuSnrState { id: 0, snr1_count: 5, snr2_count: 5 },
        ]);
        g.drain_one(SNR_MASK_SNR1);
        assert_eq!(g.spus[0].snr1_count, 4);
        assert_eq!(g.spus[0].snr2_count, 5); // SNR2 untouched
    }

    // ---- wait_outcome -----------------------------------------------

    #[test]
    fn wait_outcome_mask_zero_returns_mask_is_empty() {
        // `snr_mask % 4 == 0` — both LSBs unset.
        let g = SpuGroup::new();
        assert_eq!(wait_outcome(&g, SPU_ID_ALL, 0), WaitOutcome::MaskIsEmpty);
        assert_eq!(wait_outcome(&g, SPU_ID_ALL, 4), WaitOutcome::MaskIsEmpty);
        assert_eq!(wait_outcome(&g, SPU_ID_ALL, 8), WaitOutcome::MaskIsEmpty);
    }

    #[test]
    fn wait_outcome_spu_not_found() {
        let g = SpuGroup::with_spus(alloc::vec![SpuSnrState::new(0)]);
        assert_eq!(wait_outcome(&g, 99, SNR_MASK_SNR1), WaitOutcome::SpuNotFound);
    }

    #[test]
    fn wait_outcome_spu_id_all_with_empty_group_still_spins() {
        // C++ doesn't special-case empty groups for `umax`; `idm::select`
        // simply iterates over zero SPUs.
        let g = SpuGroup::new();
        assert!(matches!(
            wait_outcome(&g, SPU_ID_ALL, SNR_MASK_SNR1),
            WaitOutcome::WouldSpin { all_spus: true },
        ));
    }

    #[test]
    fn wait_outcome_would_spin_specific_spu() {
        let g = SpuGroup::with_spus(alloc::vec![SpuSnrState::new(42)]);
        assert!(matches!(
            wait_outcome(&g, 42, SNR_MASK_RELEVANT),
            WaitOutcome::WouldSpin { all_spus: false },
        ));
    }

    #[test]
    fn wait_outcome_spu_missing_beats_mask_check() {
        // Even if snr_mask is "empty" (0), the spu-missing check runs
        // first because C++ short-circuits left-to-right.
        let g = SpuGroup::new();
        // spu_id != umax, group empty → SpuNotFound.
        assert_eq!(wait_outcome(&g, 0, 0), WaitOutcome::SpuNotFound);
    }

    // ---- wait_for_spus_to_empty_snrs --------------------------------

    #[test]
    fn wait_returns_early_on_mask_zero() {
        let mut g = SpuGroup::with_spus(alloc::vec![
            SpuSnrState { id: 0, snr1_count: 9, snr2_count: 9 },
        ]);
        let r = wait_for_spus_to_empty_snrs(&mut g, 0, 0, 1000, true).unwrap();
        assert_eq!(r, WaitResult::ReturnedEarly);
        // Nothing drained.
        assert_eq!(g.spus[0].snr1_count, 9);
    }

    #[test]
    fn wait_returns_early_when_spu_not_found() {
        let mut g = SpuGroup::with_spus(alloc::vec![SpuSnrState::new(0)]);
        let r = wait_for_spus_to_empty_snrs(&mut g, 99, SNR_MASK_SNR1, 10, false).unwrap();
        assert_eq!(r, WaitResult::ReturnedEarly);
    }

    #[test]
    fn wait_drains_single_spu_with_helper() {
        let mut g = SpuGroup::with_spus(alloc::vec![
            SpuSnrState { id: 0, snr1_count: 3, snr2_count: 0 },
        ]);
        let r = wait_for_spus_to_empty_snrs(&mut g, 0, SNR_MASK_SNR1, 100, true).unwrap();
        assert_eq!(r, WaitResult::Drained);
        assert_eq!(g.spus[0].snr1_count, 0);
    }

    #[test]
    fn wait_drains_all_spus() {
        let mut g = SpuGroup::with_spus(alloc::vec![
            SpuSnrState { id: 0, snr1_count: 1, snr2_count: 2 },
            SpuSnrState { id: 1, snr1_count: 3, snr2_count: 0 },
        ]);
        let r = wait_for_spus_to_empty_snrs(
            &mut g, SPU_ID_ALL, SNR_MASK_RELEVANT, 100, true,
        ).unwrap();
        assert_eq!(r, WaitResult::Drained);
        assert!(!g.any_busy(SNR_MASK_RELEVANT));
    }

    #[test]
    fn wait_exhausts_cap_when_drain_disabled() {
        let mut g = SpuGroup::with_spus(alloc::vec![
            SpuSnrState { id: 0, snr1_count: 1, snr2_count: 0 },
        ]);
        // drain_per_iter=false → count never changes → cap exhausted.
        assert_eq!(
            wait_for_spus_to_empty_snrs(&mut g, 0, SNR_MASK_SNR1, 10, false).unwrap_err(),
            CELL_EINVAL,
        );
    }

    // ---- registry ---------------------------------------------------

    #[test]
    fn registry_has_single_entry() {
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 1);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "WaitForSPUsToEmptySNRs");
    }

    #[test]
    fn is_registered_helper() {
        assert!(is_registered("WaitForSPUsToEmptySNRs"));
        assert!(!is_registered("WaitForSomethingElse"));
    }

    // ---- full smoke --------------------------------------------------

    #[test]
    fn full_hle_patches_lifecycle_smoke() {
        // 1. Sonic-style workaround: wait for SNR1 + SNR2 to drain on
        //    two specific SPUs, with a `drain_one` helper simulating
        //    the SPU-side consumer.
        let mut g = SpuGroup::with_spus(alloc::vec![
            SpuSnrState { id: 0, snr1_count: 4, snr2_count: 6 },
            SpuSnrState { id: 1, snr1_count: 5, snr2_count: 3 },
        ]);

        // Decision check.
        let outcome = wait_outcome(&g, 1, SNR_MASK_RELEVANT);
        assert!(matches!(outcome, WaitOutcome::WouldSpin { all_spus: false }));

        // Spin-wait for SPU 1 only.  Loop terminates as soon as SPU 1's
        // requested registers are empty (max(5, 3) = 5 iterations).
        let r = wait_for_spus_to_empty_snrs(&mut g, 1, SNR_MASK_RELEVANT, 100, true).unwrap();
        assert_eq!(r, WaitResult::Drained);
        assert!(!g.spus[1].has_busy(SNR_MASK_RELEVANT));
        // SPU 0 was drained in lockstep by `drain_one(mask)` — after 5
        // iterations its SNR1 (started at 4) is saturated at 0, but
        // SNR2 (started at 6) still has 1 pending.  The wait only
        // cares about SPU 1 because spu_id != SPU_ID_ALL.
        assert_eq!(g.spus[0].snr1_count, 0);
        assert_eq!(g.spus[0].snr2_count, 1);

        // 2. mask=0 → return immediately, no work done.
        let mut g = SpuGroup::with_spus(alloc::vec![
            SpuSnrState { id: 0, snr1_count: 9, snr2_count: 9 },
        ]);
        let r = wait_for_spus_to_empty_snrs(&mut g, SPU_ID_ALL, 0, 100, true).unwrap();
        assert_eq!(r, WaitResult::ReturnedEarly);
        assert_eq!(g.spus[0].snr1_count, 9);
    }
}
