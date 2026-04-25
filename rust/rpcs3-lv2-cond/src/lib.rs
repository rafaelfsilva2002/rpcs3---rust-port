//! `rpcs3-lv2-cond` — condition variable LV2 syscalls.
//!
//! Ports `rpcs3/Emu/Cell/lv2/sys_cond.cpp`. Condition variables in LV2
//! are **always tied to an existing mutex** — the create syscall takes
//! the mutex id, and wait atomically releases+reacquires it around the
//! park. This crate stays single-threaded and synchronous like the
//! rest of the LV2 port: blocking operations return a [`WaitOutcome`]
//! for the emu core to interpret.
//!
//! ## Syscalls covered
//!
//! | LV2 syscall            | Rust wrapper                 |
//! |------------------------|------------------------------|
//! | `sys_cond_create`      | [`sys_cond_create`]          |
//! | `sys_cond_destroy`     | [`sys_cond_destroy`]         |
//! | `sys_cond_wait`        | [`sys_cond_wait`]            |
//! | `sys_cond_signal`      | [`sys_cond_signal`]          |
//! | `sys_cond_signal_all`  | [`sys_cond_signal_all`]      |
//! | `sys_cond_signal_to`   | [`sys_cond_signal_to`]       |
//!
//! ## Semantics reference (C++ `sys_cond.cpp`)
//!
//! * `sys_cond_create(cond_id_out, mutex_id, attr)` — requires an
//!   existing mutex; returns `ESRCH` if the mutex is gone.
//! * `sys_cond_destroy` — `EBUSY` if any thread is parked on the cv.
//! * `sys_cond_wait` — caller must own the mutex (`EPERM` otherwise);
//!   atomically enqueues waiter + releases mutex. After wake-up the
//!   mutex is reacquired before returning.
//! * `sys_cond_signal` — wakes one waiter (or no-op if queue empty);
//!   `ESRCH` if `cond_id` unknown.
//! * `sys_cond_signal_all` — wakes every waiter.
//! * `sys_cond_signal_to` — wakes specifically the thread id passed,
//!   `EPERM` if that thread is not parked on this cv.
//!
//! ## Trait plug-in model
//!
//! The actual mutex registry (owner tracking, waiter queues) lives in
//! the emu core, which also owns the PPU scheduler. This crate
//! abstracts over it via [`CondRegistry`] — a trait the core
//! implements. The syscalls in this crate are thin wrappers that
//! validate arguments and forward to the registry. A reference
//! implementation [`TestCondRegistry`] is provided for unit tests and
//! as a behavioural oracle.

use rpcs3_emu_types::CellError;

// =====================================================================
// Attribute constants — kept in sync with `rpcs3-lv2-sync`
// =====================================================================

/// `SYS_SYNC_FIFO` — waiters queued FIFO (default).
pub const PROTOCOL_FIFO: u32 = 0x01;
/// `SYS_SYNC_PRIORITY` — waiters queued by priority.
pub const PROTOCOL_PRIORITY: u32 = 0x02;
/// `SYS_SYNC_PRIORITY_INHERIT` — priority inheritance (accepted, not
/// semantically different from `PRIORITY` in this port).
pub const PROTOCOL_PRIORITY_INHERIT: u32 = 0x03;

/// Infinite timeout sentinel for `sys_cond_wait` (0 in LV2).
pub const TIMEOUT_INFINITE: u64 = 0;

// =====================================================================
// Attribute struct
// =====================================================================

/// Attributes passed to `sys_cond_create`. Matches the subset of
/// `sys_cond_attribute_t` the kernel actually reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CondAttr {
    /// `pshared` — 0x200 = SYS_SYNC_NOT_PROCESS_SHARED (default), IPC
    /// key ignored when not-shared.
    pub pshared: u32,
    pub flags: i32,
    pub ipc_key: u64,
    pub name: u64,
}

impl Default for CondAttr {
    fn default() -> Self {
        Self { pshared: 0x200, flags: 0, ipc_key: 0, name: 0 }
    }
}

// =====================================================================
// Outcome for blocking wait
// =====================================================================

/// Result of a `sys_cond_wait` attempt. The `Woken` variants are what
/// the wrapper returns to the caller once scheduling has resumed the
/// thread; `MustBlock` is the "please park me" signal the emu core
/// acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WaitOutcome {
    /// Caller enqueued on the cv; emu core must park the thread and
    /// call [`CondRegistry::cond_resume_waiter`] once a matching
    /// signal arrives.
    MustBlock,
    /// Waiter successfully released+reacquired the mutex and woke up.
    Woken,
    /// Timeout elapsed while parked (non-zero `timeout_us` passed and
    /// no signal arrived in time).
    Timeout,
}

// =====================================================================
// Trait — plugged in by the emu core
// =====================================================================

/// Registry of condition variables. The emu core owns the underlying
/// tables (cv → mutex_id + waiter queue) and provides this trait.
pub trait CondRegistry {
    /// Create a cv tied to `mutex_id`.
    ///
    /// Returns `ESRCH` if `mutex_id` is not a live mutex.
    fn cond_create(&mut self, attr: CondAttr, mutex_id: u32) -> Result<u32, CellError>;

    /// Destroy cv. Returns `EBUSY` if any thread is currently parked,
    /// `ESRCH` if the id is unknown.
    fn cond_destroy(&mut self, id: u32) -> Result<(), CellError>;

    /// Begin a wait. Emu core must verify the calling thread owns the
    /// associated mutex (returning `EPERM` otherwise) before enqueuing
    /// it and releasing the mutex. Returns [`WaitOutcome::MustBlock`]
    /// in the happy path — the caller then parks the thread.
    ///
    /// `timeout_us == 0` means infinite wait.
    fn cond_wait(
        &mut self,
        id: u32,
        tid: u32,
        timeout_us: u64,
    ) -> Result<WaitOutcome, CellError>;

    /// Called by the emu core when the parked thread has been
    /// scheduled back in. Validates the waiter is still on the queue
    /// and reacquires the mutex.
    fn cond_resume_waiter(&mut self, id: u32, tid: u32) -> Result<WaitOutcome, CellError>;

    /// Wake one waiter (the highest priority one if the mutex
    /// protocol is `PRIORITY`, else FIFO head). Returns the tid that
    /// was woken, or `None` if the queue was empty (valid, still
    /// `CELL_OK`).
    fn cond_signal(&mut self, id: u32) -> Result<Option<u32>, CellError>;

    /// Wake all waiters. Returns the tids in wake order.
    fn cond_signal_all(&mut self, id: u32) -> Result<Vec<u32>, CellError>;

    /// Wake specifically `tid`. Returns `EPERM` if `tid` is not parked
    /// on this cv.
    fn cond_signal_to(&mut self, id: u32, tid: u32) -> Result<(), CellError>;
}

// =====================================================================
// Syscalls — thin wrappers doing argument validation
// =====================================================================

fn validate_protocol(pshared: u32) -> Result<(), CellError> {
    match pshared {
        0x100 | 0x200 => Ok(()),
        _ => Err(CellError::EINVAL),
    }
}

/// `sys_cond_create(cond_id_out, mutex_id, attr)`.
#[must_use]
pub fn sys_cond_create<T: CondRegistry + ?Sized>(
    table: &mut T,
    mutex_id: u32,
    attr: CondAttr,
) -> Result<u32, CellError> {
    validate_protocol(attr.pshared)?;
    table.cond_create(attr, mutex_id)
}

/// `sys_cond_destroy(cond_id)`.
#[must_use]
pub fn sys_cond_destroy<T: CondRegistry + ?Sized>(
    table: &mut T,
    id: u32,
) -> Result<(), CellError> {
    table.cond_destroy(id)
}

/// `sys_cond_wait(cond_id, timeout_us)`.
#[must_use]
pub fn sys_cond_wait<T: CondRegistry + ?Sized>(
    table: &mut T,
    id: u32,
    tid: u32,
    timeout_us: u64,
) -> Result<WaitOutcome, CellError> {
    table.cond_wait(id, tid, timeout_us)
}

/// `sys_cond_signal(cond_id)`.
#[must_use]
pub fn sys_cond_signal<T: CondRegistry + ?Sized>(
    table: &mut T,
    id: u32,
) -> Result<Option<u32>, CellError> {
    table.cond_signal(id)
}

/// `sys_cond_signal_all(cond_id)`.
#[must_use]
pub fn sys_cond_signal_all<T: CondRegistry + ?Sized>(
    table: &mut T,
    id: u32,
) -> Result<Vec<u32>, CellError> {
    table.cond_signal_all(id)
}

/// `sys_cond_signal_to(cond_id, thread_id)`.
#[must_use]
pub fn sys_cond_signal_to<T: CondRegistry + ?Sized>(
    table: &mut T,
    id: u32,
    tid: u32,
) -> Result<(), CellError> {
    table.cond_signal_to(id, tid)
}

// =====================================================================
// Reference registry — also the behavioural oracle for tests
// =====================================================================

/// In-memory reference registry. Tracks the associated mutex id, the
/// FIFO waiter queue, and (for wait semantics) the thread currently
/// owning the mutex. The mutex ownership state is modeled internally
/// so tests can stay in a single crate without pulling `lv2-sync`.
#[derive(Debug, Default)]
pub struct TestCondRegistry {
    next_id: u32,
    conds: std::collections::BTreeMap<u32, CondSlot>,
    mutexes: std::collections::BTreeMap<u32, MutexSlot>,
    next_mutex_id: u32,
}

#[derive(Debug)]
struct CondSlot {
    attr: CondAttr,
    mutex_id: u32,
    waiters: Vec<u32>, // FIFO queue of thread ids currently parked
    // Threads that have been signalled but are still in the "resume"
    // phase waiting to reacquire the mutex. Separate from `waiters`
    // so `cond_destroy` only reports `EBUSY` for real sleepers.
    awakened: Vec<u32>,
}

#[derive(Debug)]
struct MutexSlot {
    owner: Option<u32>,
    /// Threads waiting to re-take ownership (from cond signal).
    relock_queue: Vec<u32>,
}

impl TestCondRegistry {
    /// Register a mutex so cond syscalls can target it. Returns the
    /// mutex id. The reference registry owns a minimal mutex model
    /// (owner + relock queue) so cv semantics can be validated
    /// without pulling in the full `lv2-sync` implementation.
    pub fn register_mutex(&mut self) -> u32 {
        self.next_mutex_id += 1;
        let id = 0x0800_0000 | self.next_mutex_id;
        self.mutexes.insert(id, MutexSlot { owner: None, relock_queue: Vec::new() });
        id
    }

    /// Test helper: give the mutex to `tid` (models `sys_mutex_lock`).
    pub fn mutex_take(&mut self, mutex_id: u32, tid: u32) -> Result<(), CellError> {
        let m = self.mutexes.get_mut(&mutex_id).ok_or(CellError::ESRCH)?;
        if m.owner.is_some() {
            return Err(CellError::EBUSY);
        }
        m.owner = Some(tid);
        Ok(())
    }

    /// Test helper: release the mutex (models `sys_mutex_unlock`).
    pub fn mutex_release(&mut self, mutex_id: u32, tid: u32) -> Result<(), CellError> {
        let m = self.mutexes.get_mut(&mutex_id).ok_or(CellError::ESRCH)?;
        if m.owner != Some(tid) {
            return Err(CellError::EPERM);
        }
        m.owner = None;
        Ok(())
    }

    /// Test helper: query current owner tid (None if free).
    pub fn mutex_owner(&self, mutex_id: u32) -> Option<u32> {
        self.mutexes.get(&mutex_id).and_then(|m| m.owner)
    }

    /// Inspect the attributes a cv was created with (test helper).
    pub fn cond_attr(&self, id: u32) -> Option<CondAttr> {
        self.conds.get(&id).map(|s| s.attr)
    }

    fn alloc_cond_id(&mut self) -> u32 {
        self.next_id += 1;
        // Match C++ `lv2_cond::id_base = 0x86000000`.
        0x8600_0000 | self.next_id
    }
}

impl CondRegistry for TestCondRegistry {
    fn cond_create(&mut self, attr: CondAttr, mutex_id: u32) -> Result<u32, CellError> {
        if !self.mutexes.contains_key(&mutex_id) {
            return Err(CellError::ESRCH);
        }
        let id = self.alloc_cond_id();
        self.conds.insert(
            id,
            CondSlot { attr, mutex_id, waiters: Vec::new(), awakened: Vec::new() },
        );
        Ok(id)
    }

    fn cond_destroy(&mut self, id: u32) -> Result<(), CellError> {
        let slot = self.conds.get(&id).ok_or(CellError::ESRCH)?;
        if !slot.waiters.is_empty() || !slot.awakened.is_empty() {
            return Err(CellError::EBUSY);
        }
        self.conds.remove(&id);
        Ok(())
    }

    fn cond_wait(
        &mut self,
        id: u32,
        tid: u32,
        _timeout_us: u64,
    ) -> Result<WaitOutcome, CellError> {
        let slot = self.conds.get_mut(&id).ok_or(CellError::ESRCH)?;
        let mutex_id = slot.mutex_id;

        // Caller must own the mutex.
        let m = self.mutexes.get_mut(&mutex_id).ok_or(CellError::ESRCH)?;
        if m.owner != Some(tid) {
            return Err(CellError::EPERM);
        }
        // Atomic: release mutex + enqueue waiter.
        m.owner = None;
        slot.waiters.push(tid);
        Ok(WaitOutcome::MustBlock)
    }

    fn cond_resume_waiter(&mut self, id: u32, tid: u32) -> Result<WaitOutcome, CellError> {
        let slot = self.conds.get_mut(&id).ok_or(CellError::ESRCH)?;
        let mutex_id = slot.mutex_id;

        // The waiter must have been moved to `awakened` (by a signal)
        // and be at the head of the mutex relock queue.
        let idx = slot
            .awakened
            .iter()
            .position(|&t| t == tid)
            .ok_or(CellError::EPERM)?;
        slot.awakened.remove(idx);

        let m = self.mutexes.get_mut(&mutex_id).ok_or(CellError::ESRCH)?;
        // Try to hand the mutex to this thread; if busy, push to
        // relock queue and stay parked.
        if m.owner.is_none() {
            m.owner = Some(tid);
            // Drop the thread from relock_queue if it was there.
            m.relock_queue.retain(|&t| t != tid);
            Ok(WaitOutcome::Woken)
        } else {
            if !m.relock_queue.contains(&tid) {
                m.relock_queue.push(tid);
            }
            Ok(WaitOutcome::MustBlock)
        }
    }

    fn cond_signal(&mut self, id: u32) -> Result<Option<u32>, CellError> {
        let slot = self.conds.get_mut(&id).ok_or(CellError::ESRCH)?;
        if slot.waiters.is_empty() {
            return Ok(None);
        }
        // FIFO vs priority: the test registry only models FIFO here.
        let tid = slot.waiters.remove(0);
        slot.awakened.push(tid);
        Ok(Some(tid))
    }

    fn cond_signal_all(&mut self, id: u32) -> Result<Vec<u32>, CellError> {
        let slot = self.conds.get_mut(&id).ok_or(CellError::ESRCH)?;
        let woken: Vec<u32> = slot.waiters.drain(..).collect();
        slot.awakened.extend(woken.iter().copied());
        Ok(woken)
    }

    fn cond_signal_to(&mut self, id: u32, tid: u32) -> Result<(), CellError> {
        let slot = self.conds.get_mut(&id).ok_or(CellError::ESRCH)?;
        let idx = slot
            .waiters
            .iter()
            .position(|&t| t == tid)
            .ok_or(CellError::EPERM)?;
        slot.waiters.remove(idx);
        slot.awakened.push(tid);
        Ok(())
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (TestCondRegistry, u32) {
        let mut reg = TestCondRegistry::default();
        let m = reg.register_mutex();
        (reg, m)
    }

    #[test]
    fn create_returns_id_with_base_0x86000000() {
        let (mut reg, m) = setup();
        let id = sys_cond_create(&mut reg, m, CondAttr::default()).unwrap();
        assert_eq!(id & 0xFF00_0000, 0x8600_0000);
    }

    #[test]
    fn create_rejects_unknown_mutex() {
        let mut reg = TestCondRegistry::default();
        let err = sys_cond_create(&mut reg, 0xDEAD_BEEF, CondAttr::default()).unwrap_err();
        assert_eq!(err, CellError::ESRCH);
    }

    #[test]
    fn create_rejects_bad_pshared() {
        let (mut reg, m) = setup();
        let bad = CondAttr { pshared: 0xABCD, ..CondAttr::default() };
        let err = sys_cond_create(&mut reg, m, bad).unwrap_err();
        assert_eq!(err, CellError::EINVAL);
    }

    #[test]
    fn destroy_unknown_is_esrch() {
        let mut reg = TestCondRegistry::default();
        let err = sys_cond_destroy(&mut reg, 0x8600_0001).unwrap_err();
        assert_eq!(err, CellError::ESRCH);
    }

    #[test]
    fn destroy_with_waiters_is_ebusy() {
        let (mut reg, m) = setup();
        reg.mutex_take(m, 42).unwrap();
        let cv = sys_cond_create(&mut reg, m, CondAttr::default()).unwrap();
        let outcome = sys_cond_wait(&mut reg, cv, 42, TIMEOUT_INFINITE).unwrap();
        assert_eq!(outcome, WaitOutcome::MustBlock);
        let err = sys_cond_destroy(&mut reg, cv).unwrap_err();
        assert_eq!(err, CellError::EBUSY);
    }

    #[test]
    fn wait_without_owning_mutex_is_eperm() {
        let (mut reg, m) = setup();
        let cv = sys_cond_create(&mut reg, m, CondAttr::default()).unwrap();
        let err = sys_cond_wait(&mut reg, cv, 42, 0).unwrap_err();
        assert_eq!(err, CellError::EPERM);
    }

    #[test]
    fn wait_releases_mutex_and_enqueues() {
        let (mut reg, m) = setup();
        reg.mutex_take(m, 7).unwrap();
        let cv = sys_cond_create(&mut reg, m, CondAttr::default()).unwrap();
        sys_cond_wait(&mut reg, cv, 7, 0).unwrap();
        assert_eq!(reg.mutex_owner(m), None, "mutex must be released on wait");
    }

    #[test]
    fn signal_empty_queue_is_ok_with_none() {
        let (mut reg, m) = setup();
        let cv = sys_cond_create(&mut reg, m, CondAttr::default()).unwrap();
        assert_eq!(sys_cond_signal(&mut reg, cv).unwrap(), None);
    }

    #[test]
    fn signal_unknown_is_esrch() {
        let mut reg = TestCondRegistry::default();
        assert_eq!(sys_cond_signal(&mut reg, 0x8600_0042).unwrap_err(), CellError::ESRCH);
    }

    #[test]
    fn signal_fifo_returns_head_tid() {
        let (mut reg, m) = setup();
        let cv = sys_cond_create(&mut reg, m, CondAttr::default()).unwrap();

        for tid in [10u32, 11, 12] {
            reg.mutex_take(m, tid).unwrap();
            sys_cond_wait(&mut reg, cv, tid, 0).unwrap();
        }

        assert_eq!(sys_cond_signal(&mut reg, cv).unwrap(), Some(10));
        assert_eq!(sys_cond_signal(&mut reg, cv).unwrap(), Some(11));
        assert_eq!(sys_cond_signal(&mut reg, cv).unwrap(), Some(12));
        assert_eq!(sys_cond_signal(&mut reg, cv).unwrap(), None);
    }

    #[test]
    fn signal_all_wakes_every_waiter() {
        let (mut reg, m) = setup();
        let cv = sys_cond_create(&mut reg, m, CondAttr::default()).unwrap();

        for tid in [100u32, 101, 102, 103] {
            reg.mutex_take(m, tid).unwrap();
            sys_cond_wait(&mut reg, cv, tid, 0).unwrap();
        }

        let woken = sys_cond_signal_all(&mut reg, cv).unwrap();
        assert_eq!(woken, vec![100, 101, 102, 103]);

        assert_eq!(sys_cond_signal(&mut reg, cv).unwrap(), None);
    }

    #[test]
    fn signal_to_wakes_specific_tid() {
        let (mut reg, m) = setup();
        let cv = sys_cond_create(&mut reg, m, CondAttr::default()).unwrap();

        for tid in [1u32, 2, 3] {
            reg.mutex_take(m, tid).unwrap();
            sys_cond_wait(&mut reg, cv, tid, 0).unwrap();
        }

        sys_cond_signal_to(&mut reg, cv, 2).unwrap();
        assert_eq!(sys_cond_signal(&mut reg, cv).unwrap(), Some(1));
        assert_eq!(sys_cond_signal(&mut reg, cv).unwrap(), Some(3));
    }

    #[test]
    fn signal_to_unknown_tid_is_eperm() {
        let (mut reg, m) = setup();
        let cv = sys_cond_create(&mut reg, m, CondAttr::default()).unwrap();
        reg.mutex_take(m, 1).unwrap();
        sys_cond_wait(&mut reg, cv, 1, 0).unwrap();
        let err = sys_cond_signal_to(&mut reg, cv, 999).unwrap_err();
        assert_eq!(err, CellError::EPERM);
    }

    #[test]
    fn resume_reacquires_mutex_when_free() {
        let (mut reg, m) = setup();
        let cv = sys_cond_create(&mut reg, m, CondAttr::default()).unwrap();

        reg.mutex_take(m, 42).unwrap();
        sys_cond_wait(&mut reg, cv, 42, 0).unwrap();
        // Mutex free now.
        sys_cond_signal(&mut reg, cv).unwrap();
        let outcome = reg.cond_resume_waiter(cv, 42).unwrap();
        assert_eq!(outcome, WaitOutcome::Woken);
        assert_eq!(reg.mutex_owner(m), Some(42));
    }

    #[test]
    fn resume_blocks_when_mutex_busy() {
        let (mut reg, m) = setup();
        let cv = sys_cond_create(&mut reg, m, CondAttr::default()).unwrap();

        reg.mutex_take(m, 42).unwrap();
        sys_cond_wait(&mut reg, cv, 42, 0).unwrap();
        sys_cond_signal(&mut reg, cv).unwrap();

        // Someone else grabs the mutex first.
        reg.mutex_take(m, 99).unwrap();

        let outcome = reg.cond_resume_waiter(cv, 42).unwrap();
        assert_eq!(outcome, WaitOutcome::MustBlock);
        // tid=42 must be queued on the mutex relock list.
        let m_slot = reg.mutexes.get(&m).unwrap();
        assert_eq!(m_slot.relock_queue, vec![42]);
    }

    #[test]
    fn resume_unknown_waiter_is_eperm() {
        let (mut reg, m) = setup();
        let cv = sys_cond_create(&mut reg, m, CondAttr::default()).unwrap();
        let err = reg.cond_resume_waiter(cv, 7).unwrap_err();
        assert_eq!(err, CellError::EPERM);
    }

    #[test]
    fn destroy_after_all_resumed_succeeds() {
        let (mut reg, m) = setup();
        let cv = sys_cond_create(&mut reg, m, CondAttr::default()).unwrap();

        reg.mutex_take(m, 42).unwrap();
        sys_cond_wait(&mut reg, cv, 42, 0).unwrap();
        sys_cond_signal(&mut reg, cv).unwrap();
        reg.cond_resume_waiter(cv, 42).unwrap();

        sys_cond_destroy(&mut reg, cv).unwrap();
    }

    #[test]
    fn cond_attr_round_trip() {
        let (mut reg, m) = setup();
        let attr = CondAttr { pshared: 0x100, flags: 0x55, ipc_key: 0xDEAD_BEEF_0000_0001, name: 0x12345678 };
        let cv = sys_cond_create(&mut reg, m, attr).unwrap();
        assert_eq!(reg.cond_attr(cv), Some(attr));
    }

    #[test]
    fn double_signal_all_is_idempotent_once_drained() {
        let (mut reg, m) = setup();
        let cv = sys_cond_create(&mut reg, m, CondAttr::default()).unwrap();
        reg.mutex_take(m, 1).unwrap();
        sys_cond_wait(&mut reg, cv, 1, 0).unwrap();
        assert_eq!(sys_cond_signal_all(&mut reg, cv).unwrap(), vec![1]);
        assert_eq!(sys_cond_signal_all(&mut reg, cv).unwrap(), Vec::<u32>::new());
    }
}
