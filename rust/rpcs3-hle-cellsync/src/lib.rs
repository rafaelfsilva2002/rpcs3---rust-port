//! `rpcs3-hle-cellsync` — user-mode sync primitives (mutex + barrier).
//!
//! Ports the mutex/barrier primitives from
//! `rpcs3/Emu/Cell/Modules/cellSync.cpp`. Unlike LV2 mutexes, these
//! live **entirely in guest shared memory** — the HLE side is a set
//! of helpers that atomically update a 4-byte (mutex/barrier) or
//! 32-byte (queue) control word. The kernel never parks anyone on
//! these primitives: callers busy-retry via `cellSyncMutexTryLock`,
//! or cooperate via user-space spinning.
//!
//! ## Entry points covered
//!
//! | HLE function              | Rust wrapper                |
//! |---------------------------|-----------------------------|
//! | `cellSyncMutexInitialize` | [`cell_sync_mutex_initialize`] |
//! | `cellSyncMutexLock`       | [`cell_sync_mutex_lock`]    |
//! | `cellSyncMutexTryLock`    | [`cell_sync_mutex_try_lock`]|
//! | `cellSyncMutexUnlock`     | [`cell_sync_mutex_unlock`]  |
//! | `cellSyncBarrierInitialize` | [`cell_sync_barrier_initialize`] |
//! | `cellSyncBarrierNotify`   | [`cell_sync_barrier_notify`] |
//! | `cellSyncBarrierTryNotify`| [`cell_sync_barrier_try_notify`] |
//! | `cellSyncBarrierWait`     | [`cell_sync_barrier_wait`]  |
//! | `cellSyncBarrierTryWait`  | [`cell_sync_barrier_try_wait`] |
//!
//! ## Frozen layouts (from `cellSync.h`)
//!
//! * [`SyncMutex`] — 4 bytes / 4-byte aligned, BE `rel:u16` + `acq:u16`.
//! * [`SyncBarrier`] — 4 bytes / 4-byte aligned, BE `value:i16` + `count:u16`,
//!   high bit of `value` is the "notified" marker (0x8000).

use rpcs3_emu_types::CellError;

// =====================================================================
// Frozen constants (from cellSync.h:11-36)
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const AGAIN: CellError = CellError(0x8041_0101);
    pub const INVAL: CellError = CellError(0x8041_0102);
    pub const NOSYS: CellError = CellError(0x8041_0103);
    pub const NOMEM: CellError = CellError(0x8041_0104);
    pub const SRCH: CellError = CellError(0x8041_0105);
    pub const NOENT: CellError = CellError(0x8041_0106);
    pub const NOEXEC: CellError = CellError(0x8041_0107);
    pub const DEADLK: CellError = CellError(0x8041_0108);
    pub const PERM: CellError = CellError(0x8041_0109);
    pub const BUSY: CellError = CellError(0x8041_010A);
    pub const ABORT: CellError = CellError(0x8041_010C);
    pub const FAULT: CellError = CellError(0x8041_010D);
    pub const CHILD: CellError = CellError(0x8041_010E);
    pub const STAT: CellError = CellError(0x8041_010F);
    pub const ALIGN: CellError = CellError(0x8041_0110);
    pub const NULL_POINTER: CellError = CellError(0x8041_0111);
    pub const NOT_SUPPORTED_THREAD: CellError = CellError(0x8041_0112);
}

// ---- Queue directions ----------------------------------------------

pub const QUEUE_SPU2SPU: u32 = 0;
pub const QUEUE_SPU2PPU: u32 = 1;
pub const QUEUE_PPU2SPU: u32 = 2;
pub const QUEUE_ANY2ANY: u32 = 3;

// =====================================================================
// Mutex — 4-byte ticket-lock layout, BE
// =====================================================================

/// `CellSyncMutex` — 4-byte control word. `rel` = release ticket
/// (= next-to-serve), `acq` = acquire ticket (= next-to-claim).
/// FIFO ordering is implicit in the ticket math.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct SyncMutex {
    pub rel_be: u16,
    pub acq_be: u16,
}

const _: () = {
    assert!(core::mem::size_of::<SyncMutex>() == 4);
    assert!(core::mem::align_of::<SyncMutex>() >= 2);
};

impl SyncMutex {
    #[must_use]
    pub fn rel(&self) -> u16 { u16::from_be(self.rel_be) }
    #[must_use]
    pub fn acq(&self) -> u16 { u16::from_be(self.acq_be) }
    pub fn set_rel(&mut self, v: u16) { self.rel_be = v.to_be(); }
    pub fn set_acq(&mut self, v: u16) { self.acq_be = v.to_be(); }
}

/// `cellSyncMutexInitialize(mutex)` — zero-out the control word.
#[must_use]
pub fn cell_sync_mutex_initialize(m: &mut SyncMutex) -> Result<(), CellError> {
    *m = SyncMutex::default();
    Ok(())
}

/// `cellSyncMutexTryLock(mutex)` — non-blocking lock. Returns `BUSY`
/// if another holder has the ticket, `Ok(())` if acquired.
#[must_use]
pub fn cell_sync_mutex_try_lock(m: &mut SyncMutex) -> Result<(), CellError> {
    if m.rel() != m.acq() {
        return Err(errors::BUSY);
    }
    m.set_acq(m.acq().wrapping_add(1));
    Ok(())
}

/// `cellSyncMutexLock(mutex)` — blocking lock. Draws a ticket and
/// returns its value; the caller must wait until `rel == ticket`
/// before entering the critical section. This matches the C++
/// `lock_begin()` helper.
#[must_use]
pub fn cell_sync_mutex_lock(m: &mut SyncMutex) -> Result<u16, CellError> {
    let my_ticket = m.acq();
    m.set_acq(my_ticket.wrapping_add(1));
    Ok(my_ticket)
}

/// `cellSyncMutexUnlock(mutex)` — release current ticket.
#[must_use]
pub fn cell_sync_mutex_unlock(m: &mut SyncMutex) -> Result<(), CellError> {
    m.set_rel(m.rel().wrapping_add(1));
    Ok(())
}

/// Poll helper for the blocking lock path — returns true when the
/// ticket has been reached and the caller now owns the mutex.
#[must_use]
pub fn mutex_poll_ready(m: &SyncMutex, ticket: u16) -> bool {
    m.rel() == ticket
}

// =====================================================================
// Barrier — 4-byte notify/wait layout, BE
// =====================================================================

/// `CellSyncBarrier` — 4-byte control: `value:i16` + `count:u16`.
/// When `value` hits `count`, the high bit (0x8000) is set to mark
/// "notified"; waiters then decrement `value` until it wraps to
/// -0x8000, at which point the bit is cleared and the barrier is
/// reusable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(C)]
pub struct SyncBarrier {
    pub value_be: i16,
    pub count_be: u16,
}

const _: () = {
    assert!(core::mem::size_of::<SyncBarrier>() == 4);
};

impl SyncBarrier {
    #[must_use]
    pub fn value(&self) -> i16 {
        i16::from_be_bytes(self.value_be.to_ne_bytes())
    }
    #[must_use]
    pub fn count(&self) -> u16 { u16::from_be(self.count_be) }

    fn set_value(&mut self, v: i16) {
        self.value_be = i16::from_ne_bytes(v.to_be_bytes());
    }
    fn set_count(&mut self, v: u16) { self.count_be = v.to_be(); }
}

/// `cellSyncBarrierInitialize(barrier, total_count)`.
#[must_use]
pub fn cell_sync_barrier_initialize(b: &mut SyncBarrier, total_count: u16) -> Result<(), CellError> {
    if total_count == 0 || total_count > 0x7FFF {
        return Err(errors::INVAL);
    }
    b.set_value(0);
    b.set_count(total_count);
    Ok(())
}

/// `cellSyncBarrierTryNotify(barrier)` — non-blocking notify.
/// Returns `BUSY` if the barrier is already in the "notified" state
/// (value has high bit set and not yet fully drained).
#[must_use]
pub fn cell_sync_barrier_try_notify(b: &mut SyncBarrier) -> Result<(), CellError> {
    if b.value() as u16 & 0x8000 != 0 {
        return Err(errors::BUSY);
    }
    let count = b.count() as i16;
    let new_value = b.value() + 1;
    if new_value == count {
        // Set notified marker (high bit).
        b.set_value((new_value as u16 | 0x8000) as i16);
    } else {
        b.set_value(new_value);
    }
    Ok(())
}

/// `cellSyncBarrierNotify(barrier)` — blocking variant, returns
/// `BUSY` indicating the caller should retry (no thread park — the
/// caller loops at the SPU/PPU side).
#[must_use]
pub fn cell_sync_barrier_notify(b: &mut SyncBarrier) -> Result<(), CellError> {
    cell_sync_barrier_try_notify(b)
}

/// `cellSyncBarrierTryWait(barrier)` — non-blocking wait. Returns
/// `BUSY` if not notified yet.
#[must_use]
pub fn cell_sync_barrier_try_wait(b: &mut SyncBarrier) -> Result<(), CellError> {
    if b.value() as u16 & 0x8000 == 0 {
        return Err(errors::BUSY);
    }
    let new_value = b.value() - 1;
    if new_value == -0x8000 {
        b.set_value(0);
    } else {
        b.set_value(new_value);
    }
    Ok(())
}

/// `cellSyncBarrierWait(barrier)` — blocking variant (caller loops).
#[must_use]
pub fn cell_sync_barrier_wait(b: &mut SyncBarrier) -> Result<(), CellError> {
    cell_sync_barrier_try_wait(b)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- constants ------------------------------------------------

    #[test]
    fn error_codes_byte_exact_vs_cellSync_h() {
        assert_eq!(errors::AGAIN.0, 0x8041_0101);
        assert_eq!(errors::INVAL.0, 0x8041_0102);
        assert_eq!(errors::BUSY.0, 0x8041_010A);
        assert_eq!(errors::ALIGN.0, 0x8041_0110);
        assert_eq!(errors::NOT_SUPPORTED_THREAD.0, 0x8041_0112);
    }

    #[test]
    fn queue_direction_constants_match_cpp() {
        assert_eq!(QUEUE_SPU2SPU, 0);
        assert_eq!(QUEUE_SPU2PPU, 1);
        assert_eq!(QUEUE_PPU2SPU, 2);
        assert_eq!(QUEUE_ANY2ANY, 3);
    }

    // --- mutex layout ---------------------------------------------

    #[test]
    fn mutex_is_4_bytes() {
        assert_eq!(core::mem::size_of::<SyncMutex>(), 4);
    }

    #[test]
    fn mutex_initialize_zeroes_ctrl() {
        let mut m = SyncMutex { rel_be: 0x1234u16.to_be(), acq_be: 0x5678u16.to_be() };
        cell_sync_mutex_initialize(&mut m).unwrap();
        assert_eq!(m.rel(), 0);
        assert_eq!(m.acq(), 0);
    }

    // --- mutex semantics ------------------------------------------

    #[test]
    fn try_lock_succeeds_when_rel_equals_acq() {
        let mut m = SyncMutex::default();
        cell_sync_mutex_try_lock(&mut m).unwrap();
        assert_eq!(m.acq(), 1);
        assert_eq!(m.rel(), 0);
    }

    #[test]
    fn try_lock_fails_when_someone_else_holds() {
        let mut m = SyncMutex::default();
        cell_sync_mutex_try_lock(&mut m).unwrap(); // A takes
        // Now rel=0, acq=1 — B tries.
        assert_eq!(cell_sync_mutex_try_lock(&mut m).unwrap_err(), errors::BUSY);
    }

    #[test]
    fn unlock_releases_and_lets_next_try_lock_succeed() {
        let mut m = SyncMutex::default();
        cell_sync_mutex_try_lock(&mut m).unwrap();
        cell_sync_mutex_unlock(&mut m).unwrap();
        assert_eq!(m.rel(), 1);
        assert_eq!(m.acq(), 1);
        cell_sync_mutex_try_lock(&mut m).unwrap();
    }

    #[test]
    fn blocking_lock_draws_fifo_ticket() {
        let mut m = SyncMutex::default();
        let t0 = cell_sync_mutex_lock(&mut m).unwrap();
        let t1 = cell_sync_mutex_lock(&mut m).unwrap();
        let t2 = cell_sync_mutex_lock(&mut m).unwrap();
        assert_eq!(t0, 0);
        assert_eq!(t1, 1);
        assert_eq!(t2, 2);
    }

    #[test]
    fn mutex_poll_ready_is_true_only_for_current_ticket() {
        let mut m = SyncMutex::default();
        let t0 = cell_sync_mutex_lock(&mut m).unwrap();
        let t1 = cell_sync_mutex_lock(&mut m).unwrap();
        assert!(mutex_poll_ready(&m, t0));
        assert!(!mutex_poll_ready(&m, t1));

        cell_sync_mutex_unlock(&mut m).unwrap();
        assert!(!mutex_poll_ready(&m, t0));
        assert!(mutex_poll_ready(&m, t1));
    }

    #[test]
    fn mutex_is_stored_big_endian() {
        let mut m = SyncMutex::default();
        cell_sync_mutex_lock(&mut m).unwrap(); // acq=1
        let raw = unsafe {
            core::slice::from_raw_parts(
                (&m as *const SyncMutex).cast::<u8>(),
                core::mem::size_of::<SyncMutex>(),
            )
        };
        // Layout: rel (2 BE bytes, = 0) then acq (2 BE bytes, = 1).
        assert_eq!(&raw[..4], &[0x00, 0x00, 0x00, 0x01]);
    }

    // --- barrier layout -------------------------------------------

    #[test]
    fn barrier_is_4_bytes() {
        assert_eq!(core::mem::size_of::<SyncBarrier>(), 4);
    }

    #[test]
    fn barrier_initialize_rejects_zero_count() {
        let mut b = SyncBarrier::default();
        assert_eq!(
            cell_sync_barrier_initialize(&mut b, 0).unwrap_err(),
            errors::INVAL,
        );
    }

    #[test]
    fn barrier_initialize_rejects_count_with_high_bit() {
        let mut b = SyncBarrier::default();
        assert_eq!(
            cell_sync_barrier_initialize(&mut b, 0x8000).unwrap_err(),
            errors::INVAL,
        );
    }

    // --- barrier semantics ----------------------------------------

    #[test]
    fn barrier_try_notify_increments_until_count_then_sets_marker() {
        let mut b = SyncBarrier::default();
        cell_sync_barrier_initialize(&mut b, 3).unwrap();
        cell_sync_barrier_try_notify(&mut b).unwrap();
        assert_eq!(b.value(), 1);
        assert_eq!(b.value() as u16 & 0x8000, 0);

        cell_sync_barrier_try_notify(&mut b).unwrap();
        assert_eq!(b.value(), 2);

        cell_sync_barrier_try_notify(&mut b).unwrap();
        // 3 + 0x8000 marker.
        assert_eq!(b.value() as u16, 3 | 0x8000);
    }

    #[test]
    fn barrier_try_notify_when_already_notified_is_busy() {
        let mut b = SyncBarrier::default();
        cell_sync_barrier_initialize(&mut b, 2).unwrap();
        cell_sync_barrier_try_notify(&mut b).unwrap();
        cell_sync_barrier_try_notify(&mut b).unwrap();
        // Third notify fails while barrier is in notified state.
        assert_eq!(cell_sync_barrier_try_notify(&mut b).unwrap_err(), errors::BUSY);
    }

    #[test]
    fn barrier_try_wait_before_notify_is_busy() {
        let mut b = SyncBarrier::default();
        cell_sync_barrier_initialize(&mut b, 3).unwrap();
        cell_sync_barrier_try_notify(&mut b).unwrap();
        // Only 1/3 notifies so far — wait must say busy.
        assert_eq!(cell_sync_barrier_try_wait(&mut b).unwrap_err(), errors::BUSY);
    }

    #[test]
    fn barrier_full_notify_then_wait_drains_cleanly() {
        let mut b = SyncBarrier::default();
        cell_sync_barrier_initialize(&mut b, 2).unwrap();
        cell_sync_barrier_try_notify(&mut b).unwrap();
        cell_sync_barrier_try_notify(&mut b).unwrap();

        // Both waiters succeed.
        cell_sync_barrier_try_wait(&mut b).unwrap();
        cell_sync_barrier_try_wait(&mut b).unwrap();

        // Final wait sees value == 0 (reusable) → BUSY again.
        assert_eq!(cell_sync_barrier_try_wait(&mut b).unwrap_err(), errors::BUSY);
        assert_eq!(b.value(), 0);
    }

    #[test]
    fn barrier_reusable_after_full_cycle() {
        let mut b = SyncBarrier::default();
        cell_sync_barrier_initialize(&mut b, 2).unwrap();
        cell_sync_barrier_try_notify(&mut b).unwrap();
        cell_sync_barrier_try_notify(&mut b).unwrap();
        cell_sync_barrier_try_wait(&mut b).unwrap();
        cell_sync_barrier_try_wait(&mut b).unwrap();

        // Fully drained — new cycle begins.
        cell_sync_barrier_try_notify(&mut b).unwrap();
        assert_eq!(b.value(), 1);
    }

    #[test]
    fn barrier_notify_wait_alias_blocking_variants() {
        let mut b = SyncBarrier::default();
        cell_sync_barrier_initialize(&mut b, 1).unwrap();
        cell_sync_barrier_notify(&mut b).unwrap();
        cell_sync_barrier_wait(&mut b).unwrap();
    }
}
