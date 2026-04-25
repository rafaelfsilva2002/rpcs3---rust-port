//! `rpcs3-lv2-event-flag` — 64-bit bitmask event-flag LV2 syscalls.
//!
//! Ports `rpcs3/Emu/Cell/lv2/sys_event_flag.cpp`. Event flags are a
//! 64-bit atomic bitmask paired with a waiter queue. A waiter
//! specifies a bit pattern and a mode (AND/OR) and wakes up when the
//! current pattern satisfies it. On wake-up, the waiter can
//! optionally clear the whole mask or just the bits it matched.
//!
//! ## Syscalls covered
//!
//! | LV2 syscall                  | Rust wrapper                  |
//! |------------------------------|-------------------------------|
//! | `sys_event_flag_create`      | [`sys_event_flag_create`]     |
//! | `sys_event_flag_destroy`     | [`sys_event_flag_destroy`]    |
//! | `sys_event_flag_wait`        | [`sys_event_flag_wait`]       |
//! | `sys_event_flag_trywait`     | [`sys_event_flag_trywait`]    |
//! | `sys_event_flag_set`         | [`sys_event_flag_set`]        |
//! | `sys_event_flag_clear`       | [`sys_event_flag_clear`]      |
//! | `sys_event_flag_get`         | [`sys_event_flag_get`]        |
//! | `sys_event_flag_cancel`      | [`sys_event_flag_cancel`]     |
//!
//! ## Mode flags (frozen from `sys_event_flag.h:10-17`)
//!
//! * Match mode — bits 0..=3:
//!   - `WAIT_AND = 0x01`: waiter satisfied when *all* requested bits are set.
//!   - `WAIT_OR  = 0x02`: waiter satisfied when *any* requested bit is set.
//! * Clear mode — bits 4..=7 (optional):
//!   - `WAIT_CLEAR     = 0x10`: clear only the matched bits on wake-up.
//!   - `WAIT_CLEAR_ALL = 0x20`: clear the whole mask on wake-up.

use rpcs3_emu_types::CellError;

// =====================================================================
// Constants
// =====================================================================

pub const WAIT_AND: u32 = 0x01;
pub const WAIT_OR: u32 = 0x02;

pub const WAIT_CLEAR: u32 = 0x10;
pub const WAIT_CLEAR_ALL: u32 = 0x20;

/// Single-waiter flag (only one thread may wait on this event_flag).
pub const WAITER_SINGLE: u32 = 0x10000;
/// Multiple-waiter flag (default).
pub const WAITER_MULTIPLE: u32 = 0x20000;

pub const PROTOCOL_FIFO: u32 = 0x01;
pub const PROTOCOL_PRIORITY: u32 = 0x02;

pub const TIMEOUT_INFINITE: u64 = 0;

// =====================================================================
// Attribute
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventFlagAttr {
    pub protocol: u32,
    pub pshared: u32,
    pub flags: i32,
    pub waiter_type: i32,
    pub ipc_key: u64,
    pub name: u64,
    pub initial_pattern: u64,
}

impl Default for EventFlagAttr {
    fn default() -> Self {
        Self {
            protocol: PROTOCOL_FIFO,
            pshared: 0x200,
            flags: 0,
            waiter_type: WAITER_MULTIPLE as i32,
            ipc_key: 0,
            name: 0,
            initial_pattern: 0,
        }
    }
}

// =====================================================================
// Outcome
// =====================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitOutcome {
    /// Pattern already satisfied; the waiter is unblocked immediately.
    /// The `u64` is the pattern *before* any clear op was applied,
    /// matching what `sys_event_flag_wait` writes back to `result`.
    Satisfied(u64),
    /// Not satisfied; caller must park.
    MustBlock,
    /// For `trywait` only: not satisfied (no parking).
    NotSatisfied,
}

// =====================================================================
// Mode helpers
// =====================================================================

fn validate_mode(mode: u32) -> Result<(), CellError> {
    match mode & 0xF {
        WAIT_AND | WAIT_OR => {}
        _ => return Err(CellError::EINVAL),
    }
    match mode & !0xF {
        0 | WAIT_CLEAR | WAIT_CLEAR_ALL => Ok(()),
        _ => Err(CellError::EINVAL),
    }
}

fn pattern_matches(pattern: u64, bitptn: u64, mode: u32) -> bool {
    match mode & 0xF {
        WAIT_AND => (pattern & bitptn) == bitptn,
        WAIT_OR => (pattern & bitptn) != 0,
        _ => false,
    }
}

/// Apply the clear-mode side-effect to `pattern` after a waiter has
/// matched on `bitptn`. Returns the new pattern.
fn apply_clear(pattern: u64, bitptn: u64, mode: u32) -> u64 {
    match mode & !0xF {
        WAIT_CLEAR => pattern & !bitptn,
        WAIT_CLEAR_ALL => 0,
        _ => pattern,
    }
}

// =====================================================================
// Registry trait
// =====================================================================

pub trait EventFlagRegistry {
    fn evflag_create(&mut self, attr: EventFlagAttr) -> Result<u32, CellError>;
    fn evflag_destroy(&mut self, id: u32) -> Result<(), CellError>;

    fn evflag_wait(
        &mut self,
        id: u32,
        tid: u32,
        bitptn: u64,
        mode: u32,
        timeout_us: u64,
    ) -> Result<WaitOutcome, CellError>;

    fn evflag_trywait(
        &mut self,
        id: u32,
        bitptn: u64,
        mode: u32,
    ) -> Result<WaitOutcome, CellError>;

    /// OR `bits` into the current pattern. Returns the waiters (tids)
    /// that now match and have been woken.
    fn evflag_set(&mut self, id: u32, bits: u64) -> Result<Vec<u32>, CellError>;

    /// Mask the current pattern with `bits` (i.e. `pattern &= bits`).
    fn evflag_clear(&mut self, id: u32, bits: u64) -> Result<(), CellError>;

    fn evflag_get(&self, id: u32) -> Result<u64, CellError>;

    /// Cancel: wake every waiter with outcome `CELL_ECANCELED`
    /// (emulated by returning the tids and letting the emu core
    /// inject the error into the parked syscalls).
    fn evflag_cancel(&mut self, id: u32) -> Result<Vec<u32>, CellError>;
}

// =====================================================================
// Syscalls
// =====================================================================

#[must_use]
pub fn sys_event_flag_create<T: EventFlagRegistry + ?Sized>(
    table: &mut T,
    attr: EventFlagAttr,
) -> Result<u32, CellError> {
    match attr.protocol {
        PROTOCOL_FIFO | PROTOCOL_PRIORITY => {}
        _ => return Err(CellError::EINVAL),
    }
    table.evflag_create(attr)
}

#[must_use]
pub fn sys_event_flag_destroy<T: EventFlagRegistry + ?Sized>(
    table: &mut T,
    id: u32,
) -> Result<(), CellError> {
    table.evflag_destroy(id)
}

#[must_use]
pub fn sys_event_flag_wait<T: EventFlagRegistry + ?Sized>(
    table: &mut T,
    id: u32,
    tid: u32,
    bitptn: u64,
    mode: u32,
    timeout_us: u64,
) -> Result<WaitOutcome, CellError> {
    validate_mode(mode)?;
    table.evflag_wait(id, tid, bitptn, mode, timeout_us)
}

#[must_use]
pub fn sys_event_flag_trywait<T: EventFlagRegistry + ?Sized>(
    table: &mut T,
    id: u32,
    bitptn: u64,
    mode: u32,
) -> Result<WaitOutcome, CellError> {
    validate_mode(mode)?;
    table.evflag_trywait(id, bitptn, mode)
}

#[must_use]
pub fn sys_event_flag_set<T: EventFlagRegistry + ?Sized>(
    table: &mut T,
    id: u32,
    bits: u64,
) -> Result<Vec<u32>, CellError> {
    table.evflag_set(id, bits)
}

#[must_use]
pub fn sys_event_flag_clear<T: EventFlagRegistry + ?Sized>(
    table: &mut T,
    id: u32,
    bits: u64,
) -> Result<(), CellError> {
    table.evflag_clear(id, bits)
}

#[must_use]
pub fn sys_event_flag_get<T: EventFlagRegistry + ?Sized>(
    table: &T,
    id: u32,
) -> Result<u64, CellError> {
    table.evflag_get(id)
}

#[must_use]
pub fn sys_event_flag_cancel<T: EventFlagRegistry + ?Sized>(
    table: &mut T,
    id: u32,
) -> Result<Vec<u32>, CellError> {
    table.evflag_cancel(id)
}

// =====================================================================
// Reference implementation
// =====================================================================

#[derive(Debug, Default)]
pub struct TestEventFlagRegistry {
    next_id: u32,
    flags: std::collections::BTreeMap<u32, Slot>,
}

#[derive(Debug)]
struct Slot {
    attr: EventFlagAttr,
    pattern: u64,
    waiters: Vec<Waiter>,
}

#[derive(Debug, Clone)]
struct Waiter {
    tid: u32,
    bitptn: u64,
    mode: u32,
}

impl TestEventFlagRegistry {
    fn alloc_id(&mut self) -> u32 {
        self.next_id += 1;
        // Match C++ `lv2_event_flag::id_base = 0x98000000`.
        0x9800_0000 | self.next_id
    }

    #[must_use]
    pub fn waiter_count(&self, id: u32) -> Option<usize> {
        self.flags.get(&id).map(|s| s.waiters.len())
    }
}

impl EventFlagRegistry for TestEventFlagRegistry {
    fn evflag_create(&mut self, attr: EventFlagAttr) -> Result<u32, CellError> {
        let id = self.alloc_id();
        self.flags.insert(
            id,
            Slot { attr, pattern: attr.initial_pattern, waiters: Vec::new() },
        );
        Ok(id)
    }

    fn evflag_destroy(&mut self, id: u32) -> Result<(), CellError> {
        let slot = self.flags.get(&id).ok_or(CellError::ESRCH)?;
        if !slot.waiters.is_empty() {
            return Err(CellError::EBUSY);
        }
        self.flags.remove(&id);
        Ok(())
    }

    fn evflag_wait(
        &mut self,
        id: u32,
        tid: u32,
        bitptn: u64,
        mode: u32,
        _timeout_us: u64,
    ) -> Result<WaitOutcome, CellError> {
        let slot = self.flags.get_mut(&id).ok_or(CellError::ESRCH)?;
        // Single-waiter type: reject if one is already queued.
        if slot.attr.waiter_type == WAITER_SINGLE as i32 && !slot.waiters.is_empty() {
            return Err(CellError::EPERM);
        }

        if pattern_matches(slot.pattern, bitptn, mode) {
            let snapshot = slot.pattern;
            slot.pattern = apply_clear(slot.pattern, bitptn, mode);
            return Ok(WaitOutcome::Satisfied(snapshot));
        }

        slot.waiters.push(Waiter { tid, bitptn, mode });
        Ok(WaitOutcome::MustBlock)
    }

    fn evflag_trywait(
        &mut self,
        id: u32,
        bitptn: u64,
        mode: u32,
    ) -> Result<WaitOutcome, CellError> {
        let slot = self.flags.get_mut(&id).ok_or(CellError::ESRCH)?;
        if pattern_matches(slot.pattern, bitptn, mode) {
            let snapshot = slot.pattern;
            slot.pattern = apply_clear(slot.pattern, bitptn, mode);
            Ok(WaitOutcome::Satisfied(snapshot))
        } else {
            Ok(WaitOutcome::NotSatisfied)
        }
    }

    fn evflag_set(&mut self, id: u32, bits: u64) -> Result<Vec<u32>, CellError> {
        let slot = self.flags.get_mut(&id).ok_or(CellError::ESRCH)?;
        slot.pattern |= bits;

        // Scan waiters: wake every one that now matches, in FIFO order.
        // On WAIT_CLEAR/CLEAR_ALL, the clear applies per-waiter at
        // wake-up — sequential evaluation handles the interaction
        // naturally (if the first waiter clears bits another was
        // also waiting on, that one stays parked).
        let mut woken = Vec::new();
        let mut i = 0;
        while i < slot.waiters.len() {
            let w = slot.waiters[i].clone();
            if pattern_matches(slot.pattern, w.bitptn, w.mode) {
                slot.pattern = apply_clear(slot.pattern, w.bitptn, w.mode);
                slot.waiters.remove(i);
                woken.push(w.tid);
            } else {
                i += 1;
            }
        }
        Ok(woken)
    }

    fn evflag_clear(&mut self, id: u32, bits: u64) -> Result<(), CellError> {
        let slot = self.flags.get_mut(&id).ok_or(CellError::ESRCH)?;
        slot.pattern &= bits;
        Ok(())
    }

    fn evflag_get(&self, id: u32) -> Result<u64, CellError> {
        let slot = self.flags.get(&id).ok_or(CellError::ESRCH)?;
        Ok(slot.pattern)
    }

    fn evflag_cancel(&mut self, id: u32) -> Result<Vec<u32>, CellError> {
        let slot = self.flags.get_mut(&id).ok_or(CellError::ESRCH)?;
        let tids: Vec<u32> = slot.waiters.drain(..).map(|w| w.tid).collect();
        Ok(tids)
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(initial: u64) -> (TestEventFlagRegistry, u32) {
        let mut reg = TestEventFlagRegistry::default();
        let attr = EventFlagAttr { initial_pattern: initial, ..EventFlagAttr::default() };
        let id = sys_event_flag_create(&mut reg, attr).unwrap();
        (reg, id)
    }

    // --- constants -------------------------------------------------

    #[test]
    fn constants_match_cpp_header() {
        assert_eq!(WAIT_AND, 0x01);
        assert_eq!(WAIT_OR, 0x02);
        assert_eq!(WAIT_CLEAR, 0x10);
        assert_eq!(WAIT_CLEAR_ALL, 0x20);
        assert_eq!(WAITER_SINGLE, 0x10000);
        assert_eq!(WAITER_MULTIPLE, 0x20000);
    }

    #[test]
    fn id_base_is_0x98000000() {
        let (_reg, id) = setup(0);
        assert_eq!(id & 0xFF00_0000, 0x9800_0000);
    }

    // --- mode validation ------------------------------------------

    #[test]
    fn bad_match_mode_is_einval() {
        let (mut reg, id) = setup(0xFF);
        let err = sys_event_flag_wait(&mut reg, id, 1, 0x1, 0x03, 0).unwrap_err();
        assert_eq!(err, CellError::EINVAL);
    }

    #[test]
    fn bad_clear_mode_is_einval() {
        let (mut reg, id) = setup(0xFF);
        let err = sys_event_flag_wait(&mut reg, id, 1, 0x1, WAIT_AND | 0x40, 0).unwrap_err();
        assert_eq!(err, CellError::EINVAL);
    }

    #[test]
    fn create_rejects_bad_protocol() {
        let mut reg = TestEventFlagRegistry::default();
        let attr = EventFlagAttr { protocol: 0xAB, ..EventFlagAttr::default() };
        assert_eq!(sys_event_flag_create(&mut reg, attr).unwrap_err(), CellError::EINVAL);
    }

    // --- AND / OR semantics ---------------------------------------

    #[test]
    fn wait_and_matches_when_all_bits_set() {
        let (mut reg, id) = setup(0b1011);
        let out = sys_event_flag_wait(&mut reg, id, 1, 0b0011, WAIT_AND, 0).unwrap();
        assert_eq!(out, WaitOutcome::Satisfied(0b1011));
    }

    #[test]
    fn wait_and_blocks_when_any_bit_missing() {
        let (mut reg, id) = setup(0b0001);
        let out = sys_event_flag_wait(&mut reg, id, 1, 0b0011, WAIT_AND, 0).unwrap();
        assert_eq!(out, WaitOutcome::MustBlock);
        assert_eq!(reg.waiter_count(id), Some(1));
    }

    #[test]
    fn wait_or_matches_on_any_bit() {
        let (mut reg, id) = setup(0b0100);
        let out = sys_event_flag_wait(&mut reg, id, 1, 0b0101, WAIT_OR, 0).unwrap();
        assert_eq!(out, WaitOutcome::Satisfied(0b0100));
    }

    #[test]
    fn wait_or_blocks_when_no_bits_set() {
        let (mut reg, id) = setup(0b0000);
        let out = sys_event_flag_wait(&mut reg, id, 1, 0b0101, WAIT_OR, 0).unwrap();
        assert_eq!(out, WaitOutcome::MustBlock);
    }

    // --- CLEAR modes ----------------------------------------------

    #[test]
    fn wait_clear_zeroes_only_matched_bits() {
        let (mut reg, id) = setup(0b1111);
        sys_event_flag_wait(&mut reg, id, 1, 0b0011, WAIT_AND | WAIT_CLEAR, 0).unwrap();
        assert_eq!(sys_event_flag_get(&reg, id).unwrap(), 0b1100);
    }

    #[test]
    fn wait_clear_all_zeroes_whole_pattern() {
        let (mut reg, id) = setup(0b1111);
        sys_event_flag_wait(&mut reg, id, 1, 0b0011, WAIT_AND | WAIT_CLEAR_ALL, 0).unwrap();
        assert_eq!(sys_event_flag_get(&reg, id).unwrap(), 0);
    }

    #[test]
    fn wait_without_clear_leaves_pattern_untouched() {
        let (mut reg, id) = setup(0b1111);
        sys_event_flag_wait(&mut reg, id, 1, 0b0011, WAIT_AND, 0).unwrap();
        assert_eq!(sys_event_flag_get(&reg, id).unwrap(), 0b1111);
    }

    // --- set wakes waiters ----------------------------------------

    #[test]
    fn set_wakes_matching_waiter() {
        let (mut reg, id) = setup(0);
        assert_eq!(
            sys_event_flag_wait(&mut reg, id, 42, 0b10, WAIT_OR, 0).unwrap(),
            WaitOutcome::MustBlock,
        );
        let woken = sys_event_flag_set(&mut reg, id, 0b10).unwrap();
        assert_eq!(woken, vec![42]);
        assert_eq!(reg.waiter_count(id), Some(0));
    }

    #[test]
    fn set_wakes_only_first_waiter_on_clear_steal() {
        // Two waiters want the same bit but the first has WAIT_CLEAR
        // → steals the bit; the second stays parked.
        let (mut reg, id) = setup(0);
        sys_event_flag_wait(&mut reg, id, 1, 0b1, WAIT_AND | WAIT_CLEAR, 0).unwrap();
        sys_event_flag_wait(&mut reg, id, 2, 0b1, WAIT_AND | WAIT_CLEAR, 0).unwrap();

        let woken = sys_event_flag_set(&mut reg, id, 0b1).unwrap();
        assert_eq!(woken, vec![1]);
        assert_eq!(reg.waiter_count(id), Some(1));
        assert_eq!(sys_event_flag_get(&reg, id).unwrap(), 0);
    }

    #[test]
    fn set_wakes_all_waiters_without_clear() {
        let (mut reg, id) = setup(0);
        sys_event_flag_wait(&mut reg, id, 1, 0b1, WAIT_AND, 0).unwrap();
        sys_event_flag_wait(&mut reg, id, 2, 0b1, WAIT_AND, 0).unwrap();
        sys_event_flag_wait(&mut reg, id, 3, 0b1, WAIT_AND, 0).unwrap();

        let woken = sys_event_flag_set(&mut reg, id, 0b1).unwrap();
        assert_eq!(woken, vec![1, 2, 3]);
    }

    // --- trywait --------------------------------------------------

    #[test]
    fn trywait_satisfied_clears_as_specified() {
        let (mut reg, id) = setup(0b101);
        let out = sys_event_flag_trywait(&mut reg, id, 0b101, WAIT_AND | WAIT_CLEAR).unwrap();
        assert_eq!(out, WaitOutcome::Satisfied(0b101));
        assert_eq!(sys_event_flag_get(&reg, id).unwrap(), 0);
    }

    #[test]
    fn trywait_unsatisfied_returns_not_satisfied() {
        let (mut reg, id) = setup(0b10);
        let out = sys_event_flag_trywait(&mut reg, id, 0b01, WAIT_AND).unwrap();
        assert_eq!(out, WaitOutcome::NotSatisfied);
    }

    // --- clear / get / cancel ------------------------------------

    #[test]
    fn clear_masks_pattern() {
        let (mut reg, id) = setup(0xFF);
        sys_event_flag_clear(&mut reg, id, 0x0F).unwrap();
        assert_eq!(sys_event_flag_get(&reg, id).unwrap(), 0x0F);
    }

    #[test]
    fn cancel_wakes_all_waiters() {
        let (mut reg, id) = setup(0);
        for tid in [10u32, 11, 12] {
            sys_event_flag_wait(&mut reg, id, tid, 0b1, WAIT_AND, 0).unwrap();
        }
        let cancelled = sys_event_flag_cancel(&mut reg, id).unwrap();
        assert_eq!(cancelled, vec![10, 11, 12]);
        assert_eq!(reg.waiter_count(id), Some(0));
    }

    // --- waiter types --------------------------------------------

    #[test]
    fn single_waiter_type_rejects_second_wait() {
        let mut reg = TestEventFlagRegistry::default();
        let attr = EventFlagAttr {
            waiter_type: WAITER_SINGLE as i32,
            ..EventFlagAttr::default()
        };
        let id = sys_event_flag_create(&mut reg, attr).unwrap();
        sys_event_flag_wait(&mut reg, id, 1, 0b1, WAIT_AND, 0).unwrap();
        let err = sys_event_flag_wait(&mut reg, id, 2, 0b1, WAIT_AND, 0).unwrap_err();
        assert_eq!(err, CellError::EPERM);
    }

    // --- destroy --------------------------------------------------

    #[test]
    fn destroy_with_waiters_is_ebusy() {
        let (mut reg, id) = setup(0);
        sys_event_flag_wait(&mut reg, id, 1, 0b1, WAIT_AND, 0).unwrap();
        assert_eq!(sys_event_flag_destroy(&mut reg, id).unwrap_err(), CellError::EBUSY);
    }

    #[test]
    fn destroy_empty_succeeds() {
        let (mut reg, id) = setup(0);
        sys_event_flag_destroy(&mut reg, id).unwrap();
        assert_eq!(sys_event_flag_destroy(&mut reg, id).unwrap_err(), CellError::ESRCH);
    }

    // --- initial pattern -----------------------------------------

    #[test]
    fn initial_pattern_is_preserved() {
        let (reg, id) = setup(0xDEAD_BEEF_CAFE_BABE);
        assert_eq!(sys_event_flag_get(&reg, id).unwrap(), 0xDEAD_BEEF_CAFE_BABE);
    }

    #[test]
    fn set_then_set_accumulates_bits() {
        let (mut reg, id) = setup(0);
        sys_event_flag_set(&mut reg, id, 0b0001).unwrap();
        sys_event_flag_set(&mut reg, id, 0b0010).unwrap();
        assert_eq!(sys_event_flag_get(&reg, id).unwrap(), 0b0011);
    }

    // --- edge: wait on bitptn=0 matches OR-style vacuously? ----
    // C++ rejects this with EINVAL (bitptn 0 is nonsensical). We
    // don't validate it here — emu core layer can. Just make sure
    // our semantics don't explode.

    #[test]
    fn wait_or_with_zero_pattern_blocks_forever() {
        let (mut reg, id) = setup(0xFFFF_FFFF_FFFF_FFFF);
        let out = sys_event_flag_wait(&mut reg, id, 1, 0, WAIT_OR, 0).unwrap();
        assert_eq!(out, WaitOutcome::MustBlock);
    }
}
