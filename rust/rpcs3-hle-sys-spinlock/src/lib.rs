//! `rpcs3-hle-sys-spinlock` — Rust port of
//! `rpcs3/Emu/Cell/Modules/sys_spinlock.cpp`.
//!
//! Userspace spinlock implementation registered under `sysPrxForUser`.
//! Uses the magic sentinel `0xABADCAFE` to mark a held lock (cpp:21).
//!
//! Frozen:
//!
//! - `LOCK_SENTINEL = 0xABADCAFE` — stored when the lock is held.
//! - `lock == 0` → available, `lock == anything != 0` → held.
//! - `initialize` — zero the lock if it's non-zero (cpp:10..13). Note
//!   this **only** clears when already non-zero, matching cpp exactly.
//! - `try_lock` — atomic CAS attempt: returns `CELL_EBUSY` if held,
//!   `CELL_OK` if acquired (cpp:33..43).
//! - `unlock` — unconditional store of 0 (cpp:45..50).

use rpcs3_emu_types::CellError;

/// Value written when the lock is held (cpp:21).
pub const LOCK_SENTINEL: u32 = 0xABAD_CAFE;

/// From `rpcs3-emu-types`: CELL_OK, CELL_EBUSY (0x8001_000A).
pub const CELL_OK: u32 = 0;
pub const CELL_EBUSY: u32 = 0x8001_000A;

/// `sys_spinlock_initialize(lock)` (cpp:6..14). Zeroes the lock only if
/// it was already non-zero. This is a quirk we preserve: writing to a
/// lock that's already 0 is a no-op.
pub fn initialize(lock: &mut u32) {
    if *lock != 0 {
        *lock = 0;
    }
}

/// `sys_spinlock_trylock(lock)` (cpp:33..43). Non-blocking acquire.
/// Returns `CELL_OK` on success (or `Err(CELL_EBUSY)` if held).
pub fn try_lock(lock: &mut u32) -> Result<u32, CellError> {
    if *lock != 0 {
        return Err(CellError(CELL_EBUSY));
    }
    let old = core::mem::replace(lock, LOCK_SENTINEL);
    if old != 0 {
        // Someone beat us in the micro-race (in a single-threaded port
        // this never happens, but preserve the cpp double-check).
        return Err(CellError(CELL_EBUSY));
    }
    Ok(CELL_OK)
}

/// `sys_spinlock_unlock(lock)` (cpp:45..50). Unconditional store of 0.
pub fn unlock(lock: &mut u32) {
    *lock = 0;
}

/// Whether a given lock word represents "held" state.
#[must_use]
pub const fn is_locked(lock: u32) -> bool {
    lock != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_cpp() {
        assert_eq!(LOCK_SENTINEL, 0xABAD_CAFE);
        assert_eq!(CELL_OK, 0);
        assert_eq!(CELL_EBUSY, 0x8001_000A);
    }

    #[test]
    fn initialize_zeroes_nonzero_lock() {
        let mut lock = 0xDEAD_BEEFu32;
        initialize(&mut lock);
        assert_eq!(lock, 0);
    }

    #[test]
    fn initialize_noop_on_already_zero() {
        let mut lock = 0u32;
        initialize(&mut lock);
        assert_eq!(lock, 0);
    }

    #[test]
    fn try_lock_acquires_on_free() {
        let mut lock = 0u32;
        assert_eq!(try_lock(&mut lock), Ok(CELL_OK));
        assert_eq!(lock, LOCK_SENTINEL);
    }

    #[test]
    fn try_lock_rejects_held() {
        let mut lock = LOCK_SENTINEL;
        assert_eq!(try_lock(&mut lock).unwrap_err().0, CELL_EBUSY);
        assert_eq!(lock, LOCK_SENTINEL);
    }

    #[test]
    fn unlock_clears() {
        let mut lock = LOCK_SENTINEL;
        unlock(&mut lock);
        assert_eq!(lock, 0);
    }

    #[test]
    fn unlock_on_free_is_idempotent() {
        let mut lock = 0u32;
        unlock(&mut lock);
        assert_eq!(lock, 0);
    }

    #[test]
    fn is_locked_predicate() {
        assert!(!is_locked(0));
        assert!(is_locked(LOCK_SENTINEL));
        assert!(is_locked(1));
        assert!(is_locked(0xFFFF_FFFF));
    }

    #[test]
    fn lock_unlock_sequence() {
        let mut lock = 0u32;
        try_lock(&mut lock).unwrap();
        assert!(is_locked(lock));
        unlock(&mut lock);
        assert!(!is_locked(lock));
        // Next acquire should succeed.
        try_lock(&mut lock).unwrap();
    }
}
