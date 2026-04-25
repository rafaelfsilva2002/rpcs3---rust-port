//! `rpcs3-cpu-thread` — cpu_thread state machine primitives.
//!
//! Mirrors the ABI-observable portions of `rpcs3/Emu/CPU/CPUThread.h`:
//!
//! * `atomic_bs_t<cpu_flag> state` — an atomic bitset of `cpu_flag`.
//!   Initial value is `stop | wait` (CPUThread.h:71).
//! * `is_stopped(state)` / `is_paused(state)` helpers (CPUThread.h:41/47).
//! * Thread-class discriminant derived from the high 8 bits of the
//!   thread id (CPUThread.h:117-120).
//!
//! Out of scope (Wave 4b+):
//! * `check_state()` control flow — requires scheduler wake-up semantics
//!   and reservation table integration, both of which land with
//!   `rpcs3-memory-backing`.
//! * `operator()` main loop.
//! * `try_get<T>()` downcast helpers (belong in `ppu_thread`/`spu_thread`).

use std::sync::atomic::{AtomicU32, Ordering};

use rpcs3_emu_types::{is_paused, is_stopped, CpuFlag};

/// Class-of-thread discriminant. Matches `thread_class` in CPUThread.h,
/// encoded as the top byte of the thread id (`id >> 24`).
///
/// | High byte | Class        |
/// |-----------|--------------|
/// | `0x01`    | PPU thread   |
/// | `0x02`    | SPU thread   |
/// | `0x03`    | RawSPU thread (direct-mapped) |
///
/// The exact mapping lives in each concrete thread type's `id_base`
/// (e.g. `ppu_thread::id_base == 0x0100_0000`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ThreadClass {
    Ppu = 0x01,
    Spu = 0x02,
    RawSpu = 0x03,
    Other = 0xFF,
}

impl ThreadClass {
    #[must_use]
    pub const fn from_id(id: u32) -> Self {
        match (id >> 24) as u8 {
            0x01 => Self::Ppu,
            0x02 => Self::Spu,
            0x03 => Self::RawSpu,
            _ => Self::Other,
        }
    }
}

// =====================================================================
// Atomic bitset over cpu_flag
// =====================================================================

/// Atomic equivalent of `atomic_bs_t<cpu_flag>` in RPCS3.
///
/// Stores a 32-bit bitset where bit `n` corresponds to the flag with
/// ordinal `n`. Uses `SeqCst` ordering by default to match the strong
/// ordering assumed by the C++ scheduler code; callers with stricter
/// performance needs can use the `_relaxed` variants.
#[derive(Debug, Default)]
pub struct CpuState(pub AtomicU32);

impl CpuState {
    /// Initial state used by `cpu_thread(u32)` constructor:
    /// `cpu_flag::stop + cpu_flag::wait` (CPUThread.h:71).
    #[must_use]
    pub fn initial() -> Self {
        Self(AtomicU32::new(
            CpuFlag::Stop.mask() | CpuFlag::Wait.mask(),
        ))
    }

    #[must_use]
    pub fn new(bits: u32) -> Self {
        Self(AtomicU32::new(bits))
    }

    /// Atomic snapshot of the bitset.
    pub fn load(&self) -> u32 {
        self.0.load(Ordering::SeqCst)
    }

    /// Set `flag`. Returns previous value.
    pub fn set(&self, flag: CpuFlag) -> u32 {
        self.0.fetch_or(flag.mask(), Ordering::SeqCst)
    }

    /// Clear `flag`. Returns previous value.
    pub fn clear(&self, flag: CpuFlag) -> u32 {
        self.0.fetch_and(!flag.mask(), Ordering::SeqCst)
    }

    /// Set multiple flags by bitmask. Returns previous value.
    pub fn set_mask(&self, mask: u32) -> u32 {
        self.0.fetch_or(mask, Ordering::SeqCst)
    }

    /// Clear multiple flags by bitmask. Returns previous value.
    pub fn clear_mask(&self, mask: u32) -> u32 {
        self.0.fetch_and(!mask, Ordering::SeqCst)
    }

    /// Returns `true` if `flag` is currently set.
    pub fn has(&self, flag: CpuFlag) -> bool {
        (self.load() & flag.mask()) != 0
    }

    /// Returns `true` if any of the given flags are set.
    pub fn has_any(&self, mask: u32) -> bool {
        (self.load() & mask) != 0
    }

    /// Returns `true` if the current bitset represents a stopped thread
    /// (`stop | exit | again | req_exit`).
    pub fn is_stopped(&self) -> bool {
        is_stopped(self.load())
    }

    /// Returns `true` if the current bitset represents a paused thread
    /// (`suspend | dbg_global_pause | dbg_pause` AND not stopped).
    pub fn is_paused(&self) -> bool {
        is_paused(self.load())
    }

    /// Returns `true` if the `pause` flag is present. Distinct from
    /// `is_paused()` — matches `has_pause_flag()` at CPUThread.h:111.
    pub fn has_pause_flag(&self) -> bool {
        self.has(CpuFlag::Pause)
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- ThreadClass -------------------------------------------------

    #[test]
    fn thread_class_from_id() {
        assert_eq!(ThreadClass::from_id(0x0100_0000), ThreadClass::Ppu);
        assert_eq!(ThreadClass::from_id(0x01AB_CDEF), ThreadClass::Ppu);
        assert_eq!(ThreadClass::from_id(0x0200_0000), ThreadClass::Spu);
        assert_eq!(ThreadClass::from_id(0x0300_0000), ThreadClass::RawSpu);
        assert_eq!(ThreadClass::from_id(0x00FF_FFFF), ThreadClass::Other);
        assert_eq!(ThreadClass::from_id(0xAB00_0000), ThreadClass::Other);
    }

    // -- CpuState construction --------------------------------------

    #[test]
    fn initial_state_is_stop_plus_wait() {
        let s = CpuState::initial();
        assert!(s.has(CpuFlag::Stop));
        assert!(s.has(CpuFlag::Wait));
        assert!(!s.has(CpuFlag::Exit));
        assert_eq!(s.load(), 0b101); // bits 0 and 2
    }

    #[test]
    fn new_preserves_bits() {
        let s = CpuState::new(0x42);
        assert_eq!(s.load(), 0x42);
    }

    // -- Set / clear / has ------------------------------------------

    #[test]
    fn set_and_clear_single_flag() {
        let s = CpuState::new(0);
        assert!(!s.has(CpuFlag::Exit));
        let prev = s.set(CpuFlag::Exit);
        assert_eq!(prev, 0);
        assert!(s.has(CpuFlag::Exit));
        let prev2 = s.clear(CpuFlag::Exit);
        assert_eq!(prev2, CpuFlag::Exit.mask());
        assert!(!s.has(CpuFlag::Exit));
    }

    #[test]
    fn set_is_idempotent() {
        let s = CpuState::new(0);
        s.set(CpuFlag::Pause);
        s.set(CpuFlag::Pause);
        assert_eq!(s.load(), CpuFlag::Pause.mask());
    }

    #[test]
    fn set_mask_multiple_flags() {
        let s = CpuState::new(0);
        let mask = CpuFlag::Pause.mask() | CpuFlag::Wait.mask();
        s.set_mask(mask);
        assert!(s.has(CpuFlag::Pause));
        assert!(s.has(CpuFlag::Wait));
        assert!(!s.has(CpuFlag::Exit));
    }

    #[test]
    fn clear_mask_preserves_other_bits() {
        let s = CpuState::new(0);
        s.set_mask(CpuFlag::Pause.mask() | CpuFlag::Wait.mask() | CpuFlag::Exit.mask());
        s.clear_mask(CpuFlag::Pause.mask() | CpuFlag::Wait.mask());
        assert!(!s.has(CpuFlag::Pause));
        assert!(!s.has(CpuFlag::Wait));
        assert!(s.has(CpuFlag::Exit));
    }

    #[test]
    fn has_any_matches_union() {
        let s = CpuState::new(0);
        s.set(CpuFlag::Signal);
        assert!(s.has_any(CpuFlag::Signal.mask() | CpuFlag::Exit.mask()));
        assert!(!s.has_any(CpuFlag::Exit.mask() | CpuFlag::Pause.mask()));
    }

    // -- is_stopped / is_paused semantics ---------------------------

    #[test]
    fn is_stopped_on_initial_state() {
        // Initial state has stop+wait, so is_stopped returns true.
        let s = CpuState::initial();
        assert!(s.is_stopped());
    }

    #[test]
    fn is_paused_not_when_stopped() {
        // Mix stopped + paused-like flags → stopped dominates.
        let s = CpuState::new(CpuFlag::Stop.mask() | CpuFlag::Suspend.mask());
        assert!(s.is_stopped());
        assert!(!s.is_paused());
    }

    #[test]
    fn is_paused_when_only_suspend_set() {
        let s = CpuState::new(CpuFlag::Suspend.mask());
        assert!(!s.is_stopped());
        assert!(s.is_paused());
    }

    #[test]
    fn pause_flag_is_distinct_from_paused_state() {
        // CPUThread.h:111 `has_pause_flag` is NOT equivalent to is_paused.
        let s = CpuState::new(CpuFlag::Pause.mask());
        assert!(s.has_pause_flag());
        assert!(!s.is_paused()); // Pause alone doesn't qualify per CPUThread.h:49
    }

    // -- Exit / ReqExit / Again all count as stopped ----------------

    #[test]
    fn exit_again_reqexit_all_trigger_is_stopped() {
        for flag in [CpuFlag::Exit, CpuFlag::Again, CpuFlag::ReqExit] {
            let s = CpuState::new(flag.mask());
            assert!(s.is_stopped(), "{flag:?} did not trigger is_stopped");
        }
    }

    #[test]
    fn dbg_pause_triggers_is_paused() {
        for flag in [CpuFlag::DbgPause, CpuFlag::DbgGlobalPause, CpuFlag::Suspend] {
            let s = CpuState::new(flag.mask());
            assert!(s.is_paused(), "{flag:?} did not trigger is_paused");
            assert!(!s.is_stopped());
        }
    }

    #[test]
    fn empty_state_is_neither_stopped_nor_paused() {
        let s = CpuState::new(0);
        assert!(!s.is_stopped());
        assert!(!s.is_paused());
    }
}
