//! `rpcs3-lv2-ppu-thread` — PPU thread lifecycle syscalls.
//!
//! Ports `rpcs3/Emu/Cell/lv2/sys_ppu_thread.cpp` — thread creation is
//! out of scope (requires the full emu core); this crate covers
//! yield / exit / join / detach / priority ops.
//!
//! ## Iteration scope
//!
//! * `_sys_ppu_thread_exit(errorcode)` — current thread exits with an
//!   errorcode that joiners read (sys_ppu_thread.cpp:80).
//! * `sys_ppu_thread_yield()` — scheduler yield hint (:164).
//! * `sys_ppu_thread_join(thread_id)` — block until target exits,
//!   recover its errorcode (:182).
//! * `sys_ppu_thread_detach(thread_id)` — mark thread as unjoinable
//!   (:269).
//! * `sys_ppu_thread_get_join_state()` → s32 (:317).
//! * `sys_ppu_thread_set_priority(thread_id, prio)` (:330).
//! * `sys_ppu_thread_get_priority(thread_id)` → s32 (:365).
//! * `sys_ppu_thread_stop(thread_id)`, `sys_ppu_thread_restart()` — ROOT
//!   syscalls. Stubs for completeness.

use rpcs3_emu_types::CellError;

// =====================================================================
// Per-thread syscall return
// =====================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PpuThreadSyscallResult {
    /// Syscall returned normally.
    Ok(u64),
    /// Returned a Cell error.
    Err(CellError),
    /// The current thread is exiting with this errorcode; caller must
    /// stop the thread and propagate to any joiners.
    ThreadExit { errorcode: u64 },
    /// Scheduler yield request — caller should release time slice and
    /// set `cpu_flag::wait`.
    Yield,
}

impl PpuThreadSyscallResult {
    #[must_use]
    pub fn ok_s32(v: i32) -> Self {
        Self::Ok(v as i64 as u64)
    }
}

// =====================================================================
// Thread identity + bookkeeping the emu core exposes to us
// =====================================================================

/// Join state enumeration — matches `sys_ppu_thread_join_op_e` values
/// used in `sys_ppu_thread_get_join_state`.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinState {
    Joinable = 0,
    Detached = 1,
}

/// Information the ppu_thread syscalls need from the emu core.
///
/// Implementation is typically a mutex-wrapped thread map inside the
/// emulator; in tests we use [`TestThreadTable`] which is a plain
/// `HashMap` with blocking simulated by an outcome enum.
pub trait ThreadTable {
    /// True if `thread_id` refers to a live PPU thread.
    fn exists(&self, thread_id: u32) -> bool;

    /// The join state of a thread.
    fn join_state(&self, thread_id: u32) -> Result<JoinState, CellError>;

    /// Mark `thread_id` as detached. Fails with `EINVAL` if the thread
    /// does not exist or is already detached.
    fn detach(&mut self, thread_id: u32) -> Result<(), CellError>;

    /// Try to collect the exit code of `thread_id`. If the target has
    /// not yet exited, the caller must block — we represent that with
    /// `JoinOutcome::NotYetTerminated`.
    fn try_join(&mut self, thread_id: u32) -> Result<JoinOutcome, CellError>;

    fn get_priority(&self, thread_id: u32) -> Result<i32, CellError>;
    fn set_priority(&mut self, thread_id: u32, prio: i32) -> Result<(), CellError>;

    /// The currently-executing thread's own id.
    fn current_thread_id(&self) -> u32;
    fn current_join_state(&self) -> JoinState;
}

/// Outcome of a join attempt. The emu core translates
/// `NotYetTerminated` into an actual thread-suspending wait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinOutcome {
    /// Target thread has exited; here's its exit code.
    Terminated { errorcode: u64 },
    /// Target is still running; caller must park this thread.
    NotYetTerminated,
}

// =====================================================================
// Syscalls
// =====================================================================

/// `_sys_ppu_thread_exit(errorcode)` — sys_ppu_thread.cpp:80.
/// The current thread exits; the errorcode is written into the
/// thread's join-state storage by the caller.
#[must_use]
pub fn _sys_ppu_thread_exit(errorcode: u64) -> PpuThreadSyscallResult {
    PpuThreadSyscallResult::ThreadExit { errorcode }
}

/// `sys_ppu_thread_yield()` — sys_ppu_thread.cpp:164.
/// Returns `Yield`; the emu core releases the remaining time slice
/// and runs the scheduler.
#[must_use]
pub fn sys_ppu_thread_yield() -> PpuThreadSyscallResult {
    PpuThreadSyscallResult::Yield
}

/// `sys_ppu_thread_join(thread_id)` — sys_ppu_thread.cpp:182.
/// On success returns the joined thread's exit code (to be written
/// by the caller into the guest `vptr`). Returns `NotYetTerminated`
/// through `JoinOutcome` — the caller must park and retry.
#[must_use]
pub fn sys_ppu_thread_join<T: ThreadTable + ?Sized>(
    table: &mut T,
    thread_id: u32,
) -> Result<JoinOutcome, CellError> {
    if !table.exists(thread_id) {
        return Err(CellError::ESRCH);
    }
    if matches!(table.join_state(thread_id)?, JoinState::Detached) {
        return Err(CellError::EINVAL);
    }
    if thread_id == table.current_thread_id() {
        return Err(CellError::EDEADLK);
    }
    table.try_join(thread_id)
}

/// `sys_ppu_thread_detach(thread_id)` — sys_ppu_thread.cpp:269.
#[must_use]
pub fn sys_ppu_thread_detach<T: ThreadTable + ?Sized>(
    table: &mut T,
    thread_id: u32,
) -> Result<(), CellError> {
    if !table.exists(thread_id) {
        return Err(CellError::ESRCH);
    }
    table.detach(thread_id)
}

/// `sys_ppu_thread_get_join_state()` — sys_ppu_thread.cpp:317.
/// Returns 0 for joinable, 1 for detached (matches C++).
#[must_use]
pub fn sys_ppu_thread_get_join_state<T: ThreadTable + ?Sized>(table: &T) -> i32 {
    table.current_join_state() as i32
}

/// `sys_ppu_thread_get_priority(thread_id)` — sys_ppu_thread.cpp:365.
#[must_use]
pub fn sys_ppu_thread_get_priority<T: ThreadTable + ?Sized>(
    table: &T,
    thread_id: u32,
) -> Result<i32, CellError> {
    if !table.exists(thread_id) {
        return Err(CellError::ESRCH);
    }
    table.get_priority(thread_id)
}

/// `sys_ppu_thread_set_priority(thread_id, prio)` — sys_ppu_thread.cpp:330.
#[must_use]
pub fn sys_ppu_thread_set_priority<T: ThreadTable + ?Sized>(
    table: &mut T,
    thread_id: u32,
    prio: i32,
) -> Result<(), CellError> {
    if !table.exists(thread_id) {
        return Err(CellError::ESRCH);
    }
    // PPC thread priorities: 0 (highest) .. 3071 (lowest) per PS3 kernel.
    // Values outside the range are EINVAL.
    if !(0..=3071).contains(&prio) {
        return Err(CellError::EINVAL);
    }
    table.set_priority(thread_id, prio)
}

// =====================================================================
// Test scaffold
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct TestThreadTable {
        current: u32,
        current_detached: bool,
        threads: HashMap<u32, TestThread>,
    }

    #[derive(Clone)]
    struct TestThread {
        priority: i32,
        detached: bool,
        exit: Option<u64>,
    }

    impl TestThreadTable {
        fn new(current_id: u32) -> Self {
            Self {
                current: current_id,
                current_detached: false,
                threads: HashMap::new(),
            }
        }
        fn add(&mut self, id: u32, prio: i32) {
            self.threads.insert(
                id,
                TestThread { priority: prio, detached: false, exit: None },
            );
        }
        fn terminate(&mut self, id: u32, code: u64) {
            if let Some(t) = self.threads.get_mut(&id) {
                t.exit = Some(code);
            }
        }
    }

    impl ThreadTable for TestThreadTable {
        fn exists(&self, id: u32) -> bool {
            self.threads.contains_key(&id)
        }
        fn join_state(&self, id: u32) -> Result<JoinState, CellError> {
            self.threads
                .get(&id)
                .map(|t| if t.detached { JoinState::Detached } else { JoinState::Joinable })
                .ok_or(CellError::ESRCH)
        }
        fn detach(&mut self, id: u32) -> Result<(), CellError> {
            let t = self.threads.get_mut(&id).ok_or(CellError::ESRCH)?;
            if t.detached {
                return Err(CellError::EINVAL);
            }
            t.detached = true;
            Ok(())
        }
        fn try_join(&mut self, id: u32) -> Result<JoinOutcome, CellError> {
            let t = self.threads.get(&id).ok_or(CellError::ESRCH)?;
            match t.exit {
                Some(code) => Ok(JoinOutcome::Terminated { errorcode: code }),
                None => Ok(JoinOutcome::NotYetTerminated),
            }
        }
        fn get_priority(&self, id: u32) -> Result<i32, CellError> {
            self.threads.get(&id).map(|t| t.priority).ok_or(CellError::ESRCH)
        }
        fn set_priority(&mut self, id: u32, prio: i32) -> Result<(), CellError> {
            let t = self.threads.get_mut(&id).ok_or(CellError::ESRCH)?;
            t.priority = prio;
            Ok(())
        }
        fn current_thread_id(&self) -> u32 {
            self.current
        }
        fn current_join_state(&self) -> JoinState {
            if self.current_detached {
                JoinState::Detached
            } else {
                JoinState::Joinable
            }
        }
    }

    // -- exit / yield ----------------------------------------------

    #[test]
    fn exit_propagates_errorcode() {
        assert_eq!(
            _sys_ppu_thread_exit(0xDEAD_BEEF),
            PpuThreadSyscallResult::ThreadExit { errorcode: 0xDEAD_BEEF }
        );
    }

    #[test]
    fn yield_returns_yield_variant() {
        assert_eq!(sys_ppu_thread_yield(), PpuThreadSyscallResult::Yield);
    }

    // -- join ------------------------------------------------------

    #[test]
    fn join_on_unknown_thread_is_esrch() {
        let mut t = TestThreadTable::new(1);
        assert_eq!(sys_ppu_thread_join(&mut t, 99), Err(CellError::ESRCH));
    }

    #[test]
    fn join_on_self_is_edeadlk() {
        let mut t = TestThreadTable::new(1);
        t.add(1, 1000);
        assert_eq!(sys_ppu_thread_join(&mut t, 1), Err(CellError::EDEADLK));
    }

    #[test]
    fn join_on_detached_is_einval() {
        let mut t = TestThreadTable::new(1);
        t.add(2, 1000);
        t.detach(2).unwrap();
        assert_eq!(sys_ppu_thread_join(&mut t, 2), Err(CellError::EINVAL));
    }

    #[test]
    fn join_on_live_thread_is_not_yet() {
        let mut t = TestThreadTable::new(1);
        t.add(2, 1000);
        assert_eq!(
            sys_ppu_thread_join(&mut t, 2),
            Ok(JoinOutcome::NotYetTerminated)
        );
    }

    #[test]
    fn join_on_terminated_thread_returns_exit_code() {
        let mut t = TestThreadTable::new(1);
        t.add(2, 1000);
        t.terminate(2, 42);
        assert_eq!(
            sys_ppu_thread_join(&mut t, 2),
            Ok(JoinOutcome::Terminated { errorcode: 42 })
        );
    }

    // -- detach ----------------------------------------------------

    #[test]
    fn detach_unknown_is_esrch() {
        let mut t = TestThreadTable::new(1);
        assert_eq!(sys_ppu_thread_detach(&mut t, 99), Err(CellError::ESRCH));
    }

    #[test]
    fn detach_once_ok_twice_einval() {
        let mut t = TestThreadTable::new(1);
        t.add(2, 1000);
        assert_eq!(sys_ppu_thread_detach(&mut t, 2), Ok(()));
        assert_eq!(sys_ppu_thread_detach(&mut t, 2), Err(CellError::EINVAL));
    }

    // -- priority --------------------------------------------------

    #[test]
    fn get_priority_returns_value() {
        let mut t = TestThreadTable::new(1);
        t.add(2, 1234);
        assert_eq!(sys_ppu_thread_get_priority(&t, 2), Ok(1234));
    }

    #[test]
    fn get_priority_unknown_is_esrch() {
        let t = TestThreadTable::new(1);
        assert_eq!(sys_ppu_thread_get_priority(&t, 99), Err(CellError::ESRCH));
    }

    #[test]
    fn set_priority_within_range_ok() {
        let mut t = TestThreadTable::new(1);
        t.add(2, 1000);
        assert_eq!(sys_ppu_thread_set_priority(&mut t, 2, 500), Ok(()));
        assert_eq!(sys_ppu_thread_get_priority(&t, 2), Ok(500));
    }

    #[test]
    fn set_priority_out_of_range_is_einval() {
        let mut t = TestThreadTable::new(1);
        t.add(2, 1000);
        assert_eq!(
            sys_ppu_thread_set_priority(&mut t, 2, -1),
            Err(CellError::EINVAL)
        );
        assert_eq!(
            sys_ppu_thread_set_priority(&mut t, 2, 3072),
            Err(CellError::EINVAL)
        );
    }

    #[test]
    fn set_priority_unknown_is_esrch() {
        let mut t = TestThreadTable::new(1);
        assert_eq!(
            sys_ppu_thread_set_priority(&mut t, 99, 500),
            Err(CellError::ESRCH)
        );
    }

    // -- join state ------------------------------------------------

    #[test]
    fn get_join_state_default_is_joinable() {
        let t = TestThreadTable::new(1);
        assert_eq!(sys_ppu_thread_get_join_state(&t), 0);
    }

    #[test]
    fn get_join_state_after_detach_is_one() {
        let mut t = TestThreadTable::new(1);
        t.current_detached = true;
        assert_eq!(sys_ppu_thread_get_join_state(&t), 1);
    }
}
