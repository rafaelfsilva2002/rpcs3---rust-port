//! `rpcs3-hle-sys-lwcond-user` — PS3 lightweight condition variable
//! user-mode HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/sys_lwcond_.cpp`.  A lightweight
//! condvar is always paired with a [`LwMutex`] — the firmware walks
//! three observable paths when signalling:
//!
//! 1. mutex has the `SYS_SYNC_RETRY` protocol → direct syscall (mode 2).
//! 2. caller already owns the mutex → increment `all_info` then syscall
//!    (mode 1).
//! 3. caller does not own → `trylock`; on `EBUSY` → syscall (mode 2);
//!    on success → set waiter++ + owner=`LWMUTEX_RESERVED`, then syscall
//!    (mode 3); any other trylock error → [`CELL_ESRCH`].
//!
//! [`LwCond::signal_path`] / [`LwCond::signal_all_path`] /
//! [`LwCond::signal_to_path`] expose each outcome as an [`SignalOutcome`]
//! enum so higher layers can drive the syscall half.  `wait_prepare` +
//! `wait_finish` similarly split the condvar-wait lifecycle.
//!
//! ## Entry points covered
//!
//! | C++ function          | Rust wrapper                       |
//! |-----------------------|------------------------------------|
//! | `sys_lwcond_create`   | [`LwCond::create`]                 |
//! | `sys_lwcond_destroy`  | [`LwCond::destroy`]                |
//! | `sys_lwcond_signal`   | [`LwCond::signal_path`]            |
//! | `sys_lwcond_signal_all` | [`LwCond::signal_all_path`]      |
//! | `sys_lwcond_signal_to`  | [`LwCond::signal_to_path`]       |
//! | `sys_lwcond_wait`     | [`LwCond::wait_prepare`] + [`LwCond::wait_finish`] |

use rpcs3_emu_types::CellError;
use rpcs3_hle_sys_lwmutex_user::{
    LwMutex, LWMUTEX_DEAD, LWMUTEX_FREE, LWMUTEX_RESERVED,
    SYS_SYNC_RETRY, CELL_EINVAL, CELL_EPERM, CELL_EBUSY,
    CELL_ESRCH, CELL_ETIMEDOUT,
};

pub use rpcs3_hle_sys_lwmutex_user as lwmutex;

/// `CELL_EDEADLK`.
pub const CELL_EDEADLK: CellError = CellError(0x8001_0012);

/// `SYS_SYNC_ATTR_PROTOCOL_MASK = 0xf` from sys_sync.h:23.  Used by the
/// RETRY-path detection in sys_lwcond_.cpp:59 / 139 / 208.
pub const SYS_SYNC_ATTR_PROTOCOL_MASK: u32 = 0xF;

/// Mirror of `sys_lwcond_t` — two u32 slots.  In C++ both hold guest
/// addresses; the Rust port stores the lwmutex by reference and the
/// kernel queue id as a plain `u32`.
#[derive(Debug)]
pub struct LwCond {
    /// Kernel condvar id (`lwcond_queue` in sys_lwcond_t).
    pub lwcond_queue: u32,
    /// Name packed into a u64 (matches `attr->name_u64`).
    pub name_u64: u64,
}

impl LwCond {
    /// Port of `sys_lwcond_create`.  Allocates the kernel-visible cond
    /// id and stores the attribute name.
    #[must_use]
    pub fn create(kernel_cond_id: u32, name_u64: u64) -> Self {
        Self { lwcond_queue: kernel_cond_id, name_u64 }
    }

    /// Port of `sys_lwcond_destroy`.  Clears the kernel id (marking it
    /// as `lwmutex_dead` — matches sys_lwcond_.cpp:44).
    pub fn destroy(&mut self) { self.lwcond_queue = LWMUTEX_DEAD; }

    /// Classify the signal dispatch path given the paired mutex's
    /// state.  Port of the decision tree in sys_lwcond_.cpp:57-96.
    ///
    /// `caller_tid` is the PPU id of the calling thread — used to
    /// detect the "already owns mutex" fast path.  `mode` is the kernel
    /// syscall mode the firmware will invoke (1 / 2 / 3).
    ///
    /// # Errors
    /// * [`CELL_ESRCH`] if a non-contention `trylock` error occurs.
    pub fn signal_path(lwmutex: &mut LwMutex, caller_tid: u32) -> Result<SignalOutcome, CellError> {
        // Path 1: SYS_SYNC_RETRY mask hit.
        if (lwmutex.attribute & SYS_SYNC_ATTR_PROTOCOL_MASK) == SYS_SYNC_RETRY {
            return Ok(SignalOutcome::SyscallDirect { mode: 2 });
        }
        // Path 2: caller already owns the mutex.
        if lwmutex.owner == caller_tid {
            lwmutex.all_info_waiters = lwmutex.all_info_waiters.saturating_add(1);
            return Ok(SignalOutcome::SyscallWithOwner { mode: 1 });
        }
        // Path 3: try to lock.
        match lwmutex.trylock(caller_tid) {
            Ok(()) => {
                // Success — mark as reserved + waiter++.
                lwmutex.all_info_waiters = lwmutex.all_info_waiters.saturating_add(1);
                lwmutex.owner = LWMUTEX_RESERVED;
                Ok(SignalOutcome::SyscallAfterLock { mode: 3 })
            }
            Err(e) if e == CELL_EBUSY => {
                // Contention → fall back to direct syscall mode 2.
                Ok(SignalOutcome::SyscallDirect { mode: 2 })
            }
            Err(_) => Err(CELL_ESRCH),
        }
    }

    /// Port of `sys_lwcond_signal_all` — mirrors `signal_path` but
    /// tracks the waiter count differently (it's incremented by the
    /// syscall's return value).  We reuse the [`SignalOutcome`] shape.
    ///
    /// # Errors
    /// [`CELL_ESRCH`] — same as [`Self::signal_path`].
    pub fn signal_all_path(
        lwmutex: &mut LwMutex,
        caller_tid: u32,
    ) -> Result<SignalOutcome, CellError> {
        if (lwmutex.attribute & SYS_SYNC_ATTR_PROTOCOL_MASK) == SYS_SYNC_RETRY {
            return Ok(SignalOutcome::SyscallDirect { mode: 2 });
        }
        if lwmutex.owner == caller_tid {
            // signal_all does NOT pre-increment all_info — the kernel
            // returns the waiter count which gets added later.
            return Ok(SignalOutcome::SyscallWithOwner { mode: 1 });
        }
        match lwmutex.trylock(caller_tid) {
            Ok(()) => Ok(SignalOutcome::SyscallAfterLock { mode: 1 }),
            Err(e) if e == CELL_EBUSY => Ok(SignalOutcome::SyscallDirect { mode: 2 }),
            Err(_) => Err(CELL_ESRCH),
        }
    }

    /// Port of `sys_lwcond_signal_to` — like `signal_path` but
    /// addressed to a specific PPU thread id.  Uses mode 1/2/3 with
    /// `ppu_thread_id` routing through the syscall.
    ///
    /// # Errors
    /// [`CELL_ESRCH`] if a non-contention `trylock` error occurs.
    pub fn signal_to_path(
        lwmutex: &mut LwMutex,
        caller_tid: u32,
        target_tid: u32,
    ) -> Result<SignalToOutcome, CellError> {
        if (lwmutex.attribute & SYS_SYNC_ATTR_PROTOCOL_MASK) == SYS_SYNC_RETRY {
            return Ok(SignalToOutcome { mode: 2, target_tid });
        }
        if lwmutex.owner == caller_tid {
            lwmutex.all_info_waiters = lwmutex.all_info_waiters.saturating_add(1);
            return Ok(SignalToOutcome { mode: 1, target_tid });
        }
        match lwmutex.trylock(caller_tid) {
            Ok(()) => {
                lwmutex.all_info_waiters = lwmutex.all_info_waiters.saturating_add(1);
                lwmutex.owner = LWMUTEX_RESERVED;
                Ok(SignalToOutcome { mode: 3, target_tid })
            }
            Err(e) if e == CELL_EBUSY => Ok(SignalToOutcome { mode: 2, target_tid }),
            Err(_) => Err(CELL_ESRCH),
        }
    }

    /// Port of `sys_lwcond_wait` pre-syscall phase.  Port of
    /// sys_lwcond_.cpp:286-299: checks `owner == tid`, saves the
    /// current `recursive_count`, and swaps owner → `LWMUTEX_RESERVED`
    /// with `recursive_count = 0`.
    ///
    /// # Errors
    /// * [`CELL_EPERM`] if caller does not currently own the mutex.
    pub fn wait_prepare(lwmutex: &mut LwMutex, caller_tid: u32) -> Result<WaitState, CellError> {
        if lwmutex.owner != caller_tid {
            return Err(CELL_EPERM);
        }
        let saved = WaitState {
            caller_tid,
            saved_recursive_count: lwmutex.recursive_count,
        };
        lwmutex.owner = LWMUTEX_RESERVED;
        lwmutex.recursive_count = 0;
        Ok(saved)
    }

    /// Port of `sys_lwcond_wait` post-syscall phase.  `syscall_res`
    /// mirrors the three return codes that close the three `wait`
    /// branches in sys_lwcond_.cpp:312-370:
    ///
    /// - `Ok(())`  → woken by signal; restore owner + recursive.
    /// - `Err(ESRCH)` → condvar destroyed; restore owner + recursive,
    ///   return ESRCH.
    /// - `Err(EBUSY)` / `Err(ETIMEDOUT)` → must re-lock via mutex
    ///   subsystem.  Caller delegates to [`LwMutex::lock`] and then
    ///   restores the saved recursive value.  `Err(EBUSY)` is mapped to
    ///   `Ok(())` after the lock succeeds.
    /// - `Err(EDEADLK)` → swap owner + recursive back and surface as
    ///   `CELL_ETIMEDOUT` (this is the firmware's documented recovery
    ///   — see sys_lwcond_.cpp:356-368).
    ///
    /// # Errors
    /// Propagates most syscall errors; converts `EDEADLK` → `ETIMEDOUT`.
    pub fn wait_finish(
        lwmutex: &mut LwMutex,
        state: WaitState,
        syscall_res: Result<(), CellError>,
    ) -> Result<WaitFinishOutcome, CellError> {
        match syscall_res {
            Ok(()) => {
                // Woken by signal — firmware DECREMENTS all_info.
                lwmutex.all_info_waiters = lwmutex.all_info_waiters.saturating_sub(1);
                let old = lwmutex.owner;
                lwmutex.owner = state.caller_tid;
                lwmutex.recursive_count = state.saved_recursive_count;
                if old == LWMUTEX_FREE || old == LWMUTEX_DEAD {
                    // Invariant violated — sys_lwcond_.cpp:323-326.
                    return Err(CELL_EINVAL);
                }
                Ok(WaitFinishOutcome::Woken)
            }
            Err(e) if e == CELL_ESRCH => {
                // Condvar destroyed mid-wait; firmware still restores the
                // mutex owner before bubbling ESRCH.
                let old = lwmutex.owner;
                lwmutex.owner = state.caller_tid;
                lwmutex.recursive_count = state.saved_recursive_count;
                if old == LWMUTEX_FREE || old == LWMUTEX_DEAD {
                    return Err(CELL_EINVAL);
                }
                Err(CELL_ESRCH)
            }
            Err(e) if e == CELL_EBUSY || e == CELL_ETIMEDOUT => {
                // Caller must re-lock the mutex via sys_lwmutex_lock.
                // Return an outcome that signals this — the caller runs
                // the syscall-less lock, then calls finish_relock below.
                Ok(WaitFinishOutcome::NeedsRelock {
                    saved_recursive_count: state.saved_recursive_count,
                    mapped_ok: e == CELL_EBUSY,
                })
            }
            Err(e) if e == CELL_EDEADLK => {
                // Swap owner + recursive back, surface as ETIMEDOUT.
                let old = lwmutex.owner;
                lwmutex.owner = state.caller_tid;
                lwmutex.recursive_count = state.saved_recursive_count;
                if old == LWMUTEX_FREE || old == LWMUTEX_DEAD {
                    return Err(CELL_EINVAL);
                }
                Err(CELL_ETIMEDOUT)
            }
            Err(other) => Err(other),
        }
    }

    /// After a `NeedsRelock` outcome, the caller drives
    /// [`LwMutex::lock`] back to `Acquired` and then calls this helper
    /// to restore `recursive_count`.  Mirrors sys_lwcond_.cpp:344-354.
    pub fn finish_relock(
        lwmutex: &mut LwMutex,
        saved_recursive_count: u32,
        mapped_ok: bool,
    ) -> Result<(), CellError> {
        lwmutex.recursive_count = saved_recursive_count;
        if mapped_ok {
            Ok(()) // Converted from EBUSY → CELL_OK.
        } else {
            Err(CELL_ETIMEDOUT)
        }
    }
}

/// Outcome of signal / signal_all dispatch decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalOutcome {
    /// Direct syscall — no pre-syscall mutex touch.  Used when the
    /// mutex has `SYS_SYNC_RETRY` protocol OR when the caller couldn't
    /// trylock (contention).  `mode` is the kernel syscall mode (2).
    SyscallDirect { mode: u32 },
    /// Caller already owned the mutex — firmware incremented
    /// `all_info_waiters` pre-syscall.  `mode = 1`.
    SyscallWithOwner { mode: u32 },
    /// Caller trylocked the mutex successfully — firmware set owner to
    /// `LWMUTEX_RESERVED` and bumped waiters before the syscall.
    /// `mode = 3` for signal/signal_to, `mode = 1` for signal_all.
    SyscallAfterLock { mode: u32 },
}

/// Outcome of `signal_to` — adds the target thread id to the syscall
/// arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignalToOutcome {
    pub mode: u32,
    pub target_tid: u32,
}

/// Saved state for the in-flight `wait` call.
#[derive(Debug, Clone, Copy)]
pub struct WaitState {
    pub caller_tid: u32,
    pub saved_recursive_count: u32,
}

/// Outcome of [`LwCond::wait_finish`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitFinishOutcome {
    /// Wait resolved cleanly — `owner` + `recursive_count` are already
    /// restored.
    Woken,
    /// Caller must invoke `sys_lwmutex_lock` to reacquire and then call
    /// [`LwCond::finish_relock`] to finalise.  `mapped_ok` tells the
    /// caller whether the final result is `CELL_OK` (original was
    /// `EBUSY`) or `CELL_ETIMEDOUT`.
    NeedsRelock {
        saved_recursive_count: u32,
        mapped_ok: bool,
    },
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use rpcs3_hle_sys_lwmutex_user::{
        LwMutexAttr, SYS_SYNC_NOT_RECURSIVE, SYS_SYNC_PRIORITY,
    };

    fn priority_mutex() -> LwMutex {
        let attr = LwMutexAttr {
            protocol: SYS_SYNC_PRIORITY,
            recursive: SYS_SYNC_NOT_RECURSIVE,
            name_u64: 0,
        };
        LwMutex::create(&attr, 0x1000).unwrap()
    }

    fn retry_mutex() -> LwMutex {
        let attr = LwMutexAttr {
            protocol: SYS_SYNC_RETRY,
            recursive: SYS_SYNC_NOT_RECURSIVE,
            name_u64: 0,
        };
        LwMutex::create(&attr, 0x2000).unwrap()
    }

    // ---- constants ---------------------------------------------------

    #[test]
    fn protocol_mask_byte_exact() {
        assert_eq!(SYS_SYNC_ATTR_PROTOCOL_MASK, 0xF);
    }

    #[test]
    fn re_exported_lwmutex_sentinels_still_byte_exact() {
        assert_eq!(LWMUTEX_FREE, 0xFFFF_FFFF);
        assert_eq!(LWMUTEX_DEAD, 0xFFFF_FFFE);
        assert_eq!(LWMUTEX_RESERVED, 0xFFFF_FFFD);
    }

    // ---- create / destroy -------------------------------------------

    #[test]
    fn create_stores_kernel_id_and_name() {
        let c = LwCond::create(0x4000, 0xDEAD_BEEF);
        assert_eq!(c.lwcond_queue, 0x4000);
        assert_eq!(c.name_u64, 0xDEAD_BEEF);
    }

    #[test]
    fn destroy_marks_queue_dead() {
        let mut c = LwCond::create(0x4000, 0);
        c.destroy();
        assert_eq!(c.lwcond_queue, LWMUTEX_DEAD);
    }

    // ---- signal_path ------------------------------------------------

    #[test]
    fn signal_retry_mutex_uses_direct_mode2() {
        let mut m = retry_mutex();
        let out = LwCond::signal_path(&mut m, 42).unwrap();
        assert_eq!(out, SignalOutcome::SyscallDirect { mode: 2 });
    }

    #[test]
    fn signal_owning_thread_uses_mode1_and_bumps_waiters() {
        let mut m = priority_mutex();
        m.lock(42);
        let before = m.all_info_waiters;
        let out = LwCond::signal_path(&mut m, 42).unwrap();
        assert_eq!(out, SignalOutcome::SyscallWithOwner { mode: 1 });
        assert_eq!(m.all_info_waiters, before + 1);
    }

    #[test]
    fn signal_trylock_success_uses_mode3_and_reserves() {
        let mut m = priority_mutex();
        // Mutex is free — trylock succeeds.
        let out = LwCond::signal_path(&mut m, 42).unwrap();
        assert_eq!(out, SignalOutcome::SyscallAfterLock { mode: 3 });
        assert_eq!(m.owner, LWMUTEX_RESERVED);
        assert_eq!(m.all_info_waiters, 1);
    }

    #[test]
    fn signal_trylock_contention_uses_direct_mode2() {
        let mut m = priority_mutex();
        m.lock(99); // owned by other thread
        let out = LwCond::signal_path(&mut m, 42).unwrap();
        assert_eq!(out, SignalOutcome::SyscallDirect { mode: 2 });
    }

    #[test]
    fn signal_trylock_dead_is_esrch() {
        let mut m = priority_mutex();
        m.owner = LWMUTEX_DEAD;
        let err = LwCond::signal_path(&mut m, 42).unwrap_err();
        assert_eq!(err, CELL_ESRCH);
    }

    // ---- signal_all_path --------------------------------------------

    #[test]
    fn signal_all_retry_uses_direct_mode2() {
        let mut m = retry_mutex();
        let out = LwCond::signal_all_path(&mut m, 42).unwrap();
        assert_eq!(out, SignalOutcome::SyscallDirect { mode: 2 });
    }

    #[test]
    fn signal_all_owner_uses_mode1_no_waiter_bump() {
        let mut m = priority_mutex();
        m.lock(42);
        let before = m.all_info_waiters;
        let out = LwCond::signal_all_path(&mut m, 42).unwrap();
        assert_eq!(out, SignalOutcome::SyscallWithOwner { mode: 1 });
        // signal_all does NOT pre-bump waiters — count added later from
        // syscall return.
        assert_eq!(m.all_info_waiters, before);
    }

    #[test]
    fn signal_all_after_lock_uses_mode1() {
        let mut m = priority_mutex();
        let out = LwCond::signal_all_path(&mut m, 42).unwrap();
        assert_eq!(out, SignalOutcome::SyscallAfterLock { mode: 1 });
        // signal_all also doesn't mark RESERVED — owner stays as caller.
        assert_eq!(m.owner, 42);
    }

    #[test]
    fn signal_all_dead_is_esrch() {
        let mut m = priority_mutex();
        m.owner = LWMUTEX_DEAD;
        assert_eq!(LwCond::signal_all_path(&mut m, 42).unwrap_err(), CELL_ESRCH);
    }

    // ---- signal_to_path ---------------------------------------------

    #[test]
    fn signal_to_retry_uses_mode2_with_target() {
        let mut m = retry_mutex();
        let out = LwCond::signal_to_path(&mut m, 42, 99).unwrap();
        assert_eq!(out, SignalToOutcome { mode: 2, target_tid: 99 });
    }

    #[test]
    fn signal_to_owner_uses_mode1_with_target() {
        let mut m = priority_mutex();
        m.lock(42);
        let out = LwCond::signal_to_path(&mut m, 42, 99).unwrap();
        assert_eq!(out, SignalToOutcome { mode: 1, target_tid: 99 });
        assert_eq!(m.all_info_waiters, 1);
    }

    #[test]
    fn signal_to_after_lock_uses_mode3_with_target() {
        let mut m = priority_mutex();
        let out = LwCond::signal_to_path(&mut m, 42, 99).unwrap();
        assert_eq!(out, SignalToOutcome { mode: 3, target_tid: 99 });
        assert_eq!(m.owner, LWMUTEX_RESERVED);
    }

    #[test]
    fn signal_to_contention_uses_mode2() {
        let mut m = priority_mutex();
        m.lock(42);
        let out = LwCond::signal_to_path(&mut m, 99, 7).unwrap();
        assert_eq!(out, SignalToOutcome { mode: 2, target_tid: 7 });
    }

    // ---- wait_prepare -----------------------------------------------

    #[test]
    fn wait_prepare_non_owner_is_eperm() {
        let mut m = priority_mutex();
        m.lock(42);
        let err = LwCond::wait_prepare(&mut m, 99).unwrap_err();
        assert_eq!(err, CELL_EPERM);
    }

    #[test]
    fn wait_prepare_saves_recursive_and_reserves() {
        let mut m = priority_mutex();
        m.lock(42);
        m.recursive_count = 3; // pretend we locked recursively
        let state = LwCond::wait_prepare(&mut m, 42).unwrap();
        assert_eq!(state.saved_recursive_count, 3);
        assert_eq!(m.owner, LWMUTEX_RESERVED);
        assert_eq!(m.recursive_count, 0);
    }

    // ---- wait_finish ------------------------------------------------

    #[test]
    fn wait_finish_ok_restores_and_decrements() {
        let mut m = priority_mutex();
        m.lock(42);
        m.all_info_waiters = 1;
        let state = LwCond::wait_prepare(&mut m, 42).unwrap();
        // Syscall returns Ok — woken by signal.
        let outcome = LwCond::wait_finish(&mut m, state, Ok(())).unwrap();
        assert_eq!(outcome, WaitFinishOutcome::Woken);
        assert_eq!(m.owner, 42);
        assert_eq!(m.all_info_waiters, 0);
    }

    #[test]
    fn wait_finish_esrch_restores_and_bubbles() {
        let mut m = priority_mutex();
        m.lock(42);
        let state = LwCond::wait_prepare(&mut m, 42).unwrap();
        let err = LwCond::wait_finish(&mut m, state, Err(CELL_ESRCH)).unwrap_err();
        assert_eq!(err, CELL_ESRCH);
        // Owner is still restored.
        assert_eq!(m.owner, 42);
    }

    #[test]
    fn wait_finish_ebusy_returns_needs_relock_ok() {
        let mut m = priority_mutex();
        m.lock(42);
        let state = LwCond::wait_prepare(&mut m, 42).unwrap();
        let outcome = LwCond::wait_finish(&mut m, state, Err(CELL_EBUSY)).unwrap();
        match outcome {
            WaitFinishOutcome::NeedsRelock { mapped_ok: true, .. } => {}
            other => panic!("expected NeedsRelock{{mapped_ok=true}}, got {other:?}"),
        }
    }

    #[test]
    fn wait_finish_etimedout_returns_needs_relock_fail() {
        let mut m = priority_mutex();
        m.lock(42);
        let state = LwCond::wait_prepare(&mut m, 42).unwrap();
        let outcome = LwCond::wait_finish(&mut m, state, Err(CELL_ETIMEDOUT)).unwrap();
        match outcome {
            WaitFinishOutcome::NeedsRelock { mapped_ok: false, .. } => {}
            other => panic!("expected NeedsRelock{{mapped_ok=false}}, got {other:?}"),
        }
    }

    #[test]
    fn wait_finish_edeadlk_maps_to_etimedout() {
        let mut m = priority_mutex();
        m.lock(42);
        let state = LwCond::wait_prepare(&mut m, 42).unwrap();
        let err = LwCond::wait_finish(&mut m, state, Err(CELL_EDEADLK)).unwrap_err();
        assert_eq!(err, CELL_ETIMEDOUT);
        assert_eq!(m.owner, 42);
    }

    #[test]
    fn finish_relock_ok_restores_recursive() {
        let mut m = priority_mutex();
        m.lock(42);
        LwCond::finish_relock(&mut m, 7, true).unwrap();
        assert_eq!(m.recursive_count, 7);
    }

    #[test]
    fn finish_relock_timeout_returns_etimedout() {
        let mut m = priority_mutex();
        m.lock(42);
        let err = LwCond::finish_relock(&mut m, 7, false).unwrap_err();
        assert_eq!(err, CELL_ETIMEDOUT);
        assert_eq!(m.recursive_count, 7);
    }

    // ---- full smoke --------------------------------------------------

    #[test]
    fn full_lwcond_lifecycle_smoke() {
        // 1. Create a paired mutex + condvar.
        let mut m = priority_mutex();
        let mut c = LwCond::create(0x4000, 0xDEAD_1234);

        // 2. Thread 42 locks and prepares to wait.
        m.lock(42);
        m.recursive_count = 2;
        let state = LwCond::wait_prepare(&mut m, 42).unwrap();
        assert_eq!(state.saved_recursive_count, 2);
        assert_eq!(m.owner, LWMUTEX_RESERVED);
        assert_eq!(m.recursive_count, 0);

        // 3. Meanwhile another thread signals — but the mutex is
        // reserved, trylock succeeds (we coerce owner to FREE for the
        // test, since the firmware would drop reservation on the kernel
        // side).  This is unrealistic but demonstrates the signal paths
        // in isolation.
        let mut other_m = priority_mutex();
        let sig = LwCond::signal_path(&mut other_m, 99).unwrap();
        assert_eq!(sig, SignalOutcome::SyscallAfterLock { mode: 3 });

        // 4. Kernel wakes thread 42.
        m.all_info_waiters = 1;
        let outcome = LwCond::wait_finish(&mut m, state, Ok(())).unwrap();
        assert_eq!(outcome, WaitFinishOutcome::Woken);
        assert_eq!(m.owner, 42);
        assert_eq!(m.recursive_count, 2);

        // 5. Clean up the condvar.
        c.destroy();
        assert_eq!(c.lwcond_queue, LWMUTEX_DEAD);
    }

    #[test]
    fn full_lwcond_timeout_flow_smoke() {
        let mut m = priority_mutex();
        m.lock(42);
        m.recursive_count = 5;

        let state = LwCond::wait_prepare(&mut m, 42).unwrap();
        // Syscall times out — firmware needs to re-lock.
        let outcome = LwCond::wait_finish(&mut m, state, Err(CELL_ETIMEDOUT)).unwrap();
        let (saved, mapped_ok) = match outcome {
            WaitFinishOutcome::NeedsRelock { saved_recursive_count, mapped_ok } => {
                (saved_recursive_count, mapped_ok)
            }
            other => panic!("expected NeedsRelock, got {other:?}"),
        };
        assert_eq!(saved, 5);
        assert!(!mapped_ok);

        // Caller pretends to sys_lwmutex_lock (simulated here):
        m.owner = 42;

        // Then finalises.
        let err = LwCond::finish_relock(&mut m, saved, mapped_ok).unwrap_err();
        assert_eq!(err, CELL_ETIMEDOUT);
        assert_eq!(m.recursive_count, 5);
    }
}
