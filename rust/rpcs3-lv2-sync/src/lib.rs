//! `rpcs3-lv2-sync` — mutex + semaphore LV2 syscalls.
//!
//! Ports the relevant portions of `rpcs3/Emu/Cell/lv2/sys_mutex.cpp`
//! and `sys_semaphore.cpp`. Condition variables and event queues are
//! deferred to a later iteration (they require mutex tie-ins and
//! deeper scheduler integration).
//!
//! ## Scope
//!
//! | Syscall                  | Crate fn                   |
//! |--------------------------|----------------------------|
//! | sys_mutex_create         | [`sys_mutex_create`]       |
//! | sys_mutex_destroy        | [`sys_mutex_destroy`]      |
//! | sys_mutex_lock           | [`sys_mutex_lock`]         |
//! | sys_mutex_unlock         | [`sys_mutex_unlock`]       |
//! | sys_mutex_trylock        | [`sys_mutex_trylock`]      |
//! | sys_semaphore_create     | [`sys_semaphore_create`]   |
//! | sys_semaphore_destroy    | [`sys_semaphore_destroy`]  |
//! | sys_semaphore_post       | [`sys_semaphore_post`]     |
//! | sys_semaphore_wait       | [`sys_semaphore_wait`]     |
//! | sys_semaphore_trywait    | [`sys_semaphore_trywait`]  |
//! | sys_semaphore_get_value  | [`sys_semaphore_get_value`]|
//!
//! ## Blocking model
//!
//! Blocking syscalls (`lock`, `wait`) return a [`BlockOutcome`] enum
//! instead of actually parking. The emulator core is the one that
//! knows how to suspend/resume PPU threads — this crate just reports
//! whether the operation completed immediately or the caller must
//! block.

use rpcs3_emu_types::CellError;

// =====================================================================
// Attribute constants
// =====================================================================

/// `SYS_SYNC_FIFO` — waiters queued FIFO (default).
pub const PROTOCOL_FIFO: u32 = 0x01;
/// `SYS_SYNC_PRIORITY` — waiters queued by priority.
pub const PROTOCOL_PRIORITY: u32 = 0x02;
/// `SYS_SYNC_PRIORITY_INHERIT` — priority inheritance (not modelled
/// here; we accept the flag and behave like `PRIORITY`).
pub const PROTOCOL_PRIORITY_INHERIT: u32 = 0x03;

/// `SYS_SYNC_NOT_RECURSIVE` — default for mutex.
pub const RECURSIVE_NO: u32 = 0x10;
/// `SYS_SYNC_RECURSIVE` — mutex can be locked re-entrantly by owner.
pub const RECURSIVE_YES: u32 = 0x20;

/// Infinite timeout sentinel for `lock` / `wait` (0 in LV2 convention).
pub const TIMEOUT_INFINITE: u64 = 0;

// =====================================================================
// Attributes
// =====================================================================

/// Attributes passed to `sys_mutex_create` — subset that matters for
/// behavior we model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MutexAttr {
    pub protocol: u32, // FIFO/PRIORITY/PRIORITY_INHERIT
    pub recursive: bool,
}

impl Default for MutexAttr {
    fn default() -> Self {
        Self { protocol: PROTOCOL_FIFO, recursive: false }
    }
}

/// Attributes for `sys_semaphore_create`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemaAttr {
    pub protocol: u32,
}

impl Default for SemaAttr {
    fn default() -> Self {
        Self { protocol: PROTOCOL_FIFO }
    }
}

// =====================================================================
// Outcomes for blocking calls
// =====================================================================

/// Result of an attempted lock/wait. When `MustBlock`, the caller
/// parks the calling thread on the primitive's waiter queue and
/// retries when woken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockOutcome {
    /// Succeeded immediately.
    Acquired,
    /// Would block — caller must park.
    MustBlock,
    /// Timeout expired while parked. Only returned by the caller
    /// after it parks and the scheduler decides the budget is up.
    Timeout,
}

// =====================================================================
// SyncTable trait — registries owned by the emu core
// =====================================================================

pub trait SyncTable {
    // ---- Mutex --------------------------------------------------

    fn mutex_create(&mut self, attr: MutexAttr) -> Result<u32, CellError>;
    fn mutex_destroy(&mut self, id: u32) -> Result<(), CellError>;
    fn mutex_lock(&mut self, id: u32, tid: u32) -> Result<BlockOutcome, CellError>;
    fn mutex_trylock(&mut self, id: u32, tid: u32) -> Result<(), CellError>;
    fn mutex_unlock(&mut self, id: u32, tid: u32) -> Result<(), CellError>;

    // ---- Semaphore ---------------------------------------------

    fn sema_create(
        &mut self,
        attr: SemaAttr,
        initial: i32,
        max: i32,
    ) -> Result<u32, CellError>;
    fn sema_destroy(&mut self, id: u32) -> Result<(), CellError>;
    fn sema_post(&mut self, id: u32, count: i32) -> Result<(), CellError>;
    fn sema_wait(&mut self, id: u32) -> Result<BlockOutcome, CellError>;
    fn sema_trywait(&mut self, id: u32) -> Result<(), CellError>;
    fn sema_get_value(&self, id: u32) -> Result<i32, CellError>;
}

// =====================================================================
// Syscalls (thin wrappers doing argument validation)
// =====================================================================

/// `sys_mutex_create(mutex_id_out, attr)`.
#[must_use]
pub fn sys_mutex_create<T: SyncTable + ?Sized>(
    table: &mut T,
    attr: MutexAttr,
) -> Result<u32, CellError> {
    validate_protocol(attr.protocol)?;
    table.mutex_create(attr)
}

/// `sys_mutex_destroy(mutex_id)`.
#[must_use]
pub fn sys_mutex_destroy<T: SyncTable + ?Sized>(
    table: &mut T,
    id: u32,
) -> Result<(), CellError> {
    table.mutex_destroy(id)
}

/// `sys_mutex_lock(mutex_id, timeout_us)` — returns `BlockOutcome`.
#[must_use]
pub fn sys_mutex_lock<T: SyncTable + ?Sized>(
    table: &mut T,
    id: u32,
    tid: u32,
    _timeout_us: u64,
) -> Result<BlockOutcome, CellError> {
    table.mutex_lock(id, tid)
}

/// `sys_mutex_unlock(mutex_id)`.
#[must_use]
pub fn sys_mutex_unlock<T: SyncTable + ?Sized>(
    table: &mut T,
    id: u32,
    tid: u32,
) -> Result<(), CellError> {
    table.mutex_unlock(id, tid)
}

/// `sys_mutex_trylock(mutex_id)`.
#[must_use]
pub fn sys_mutex_trylock<T: SyncTable + ?Sized>(
    table: &mut T,
    id: u32,
    tid: u32,
) -> Result<(), CellError> {
    table.mutex_trylock(id, tid)
}

/// `sys_semaphore_create(sem_id_out, attr, initial_val, max_val)`.
#[must_use]
pub fn sys_semaphore_create<T: SyncTable + ?Sized>(
    table: &mut T,
    attr: SemaAttr,
    initial: i32,
    max: i32,
) -> Result<u32, CellError> {
    validate_protocol(attr.protocol)?;
    if max <= 0 || initial < 0 || initial > max {
        return Err(CellError::EINVAL);
    }
    table.sema_create(attr, initial, max)
}

/// `sys_semaphore_destroy(sem_id)`.
#[must_use]
pub fn sys_semaphore_destroy<T: SyncTable + ?Sized>(
    table: &mut T,
    id: u32,
) -> Result<(), CellError> {
    table.sema_destroy(id)
}

/// `sys_semaphore_post(sem_id, val)`.
#[must_use]
pub fn sys_semaphore_post<T: SyncTable + ?Sized>(
    table: &mut T,
    id: u32,
    val: i32,
) -> Result<(), CellError> {
    if val < 0 {
        return Err(CellError::EINVAL);
    }
    table.sema_post(id, val)
}

/// `sys_semaphore_wait(sem_id, timeout_us)`.
#[must_use]
pub fn sys_semaphore_wait<T: SyncTable + ?Sized>(
    table: &mut T,
    id: u32,
    _timeout_us: u64,
) -> Result<BlockOutcome, CellError> {
    table.sema_wait(id)
}

/// `sys_semaphore_trywait(sem_id)`.
#[must_use]
pub fn sys_semaphore_trywait<T: SyncTable + ?Sized>(
    table: &mut T,
    id: u32,
) -> Result<(), CellError> {
    table.sema_trywait(id)
}

/// `sys_semaphore_get_value(sem_id)`.
#[must_use]
pub fn sys_semaphore_get_value<T: SyncTable + ?Sized>(
    table: &T,
    id: u32,
) -> Result<i32, CellError> {
    table.sema_get_value(id)
}

// =====================================================================
// Shared validation
// =====================================================================

fn validate_protocol(proto: u32) -> Result<(), CellError> {
    if !matches!(
        proto,
        PROTOCOL_FIFO | PROTOCOL_PRIORITY | PROTOCOL_PRIORITY_INHERIT
    ) {
        return Err(CellError::EINVAL);
    }
    Ok(())
}

// =====================================================================
// Tests — with an in-memory reference SyncTable
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, VecDeque};

    #[derive(Default)]
    struct TestTable {
        mutexes: HashMap<u32, TestMutex>,
        semas: HashMap<u32, TestSema>,
        next_id: u32,
    }

    #[derive(Default)]
    struct TestMutex {
        attr: MutexAttr,
        owner: Option<u32>,
        recursive_count: u32,
        waiters: VecDeque<u32>,
    }

    #[derive(Default)]
    #[allow(dead_code)]
    struct TestSema {
        attr: SemaAttr,
        max: i32,
        value: i32,
        waiters: VecDeque<u32>,
    }

    impl TestTable {
        fn alloc_id(&mut self) -> u32 {
            self.next_id += 1;
            self.next_id
        }
    }

    impl SyncTable for TestTable {
        fn mutex_create(&mut self, attr: MutexAttr) -> Result<u32, CellError> {
            let id = self.alloc_id();
            self.mutexes.insert(id, TestMutex { attr, ..Default::default() });
            Ok(id)
        }
        fn mutex_destroy(&mut self, id: u32) -> Result<(), CellError> {
            let m = self.mutexes.get(&id).ok_or(CellError::ESRCH)?;
            if !m.waiters.is_empty() {
                return Err(CellError::EBUSY);
            }
            if m.owner.is_some() {
                return Err(CellError::EBUSY);
            }
            self.mutexes.remove(&id);
            Ok(())
        }
        fn mutex_lock(&mut self, id: u32, tid: u32) -> Result<BlockOutcome, CellError> {
            let m = self.mutexes.get_mut(&id).ok_or(CellError::ESRCH)?;
            match m.owner {
                None => {
                    m.owner = Some(tid);
                    m.recursive_count = 1;
                    Ok(BlockOutcome::Acquired)
                }
                Some(owner) if owner == tid => {
                    if m.attr.recursive {
                        m.recursive_count += 1;
                        Ok(BlockOutcome::Acquired)
                    } else {
                        Err(CellError::EDEADLK)
                    }
                }
                Some(_) => {
                    m.waiters.push_back(tid);
                    Ok(BlockOutcome::MustBlock)
                }
            }
        }
        fn mutex_trylock(&mut self, id: u32, tid: u32) -> Result<(), CellError> {
            let m = self.mutexes.get_mut(&id).ok_or(CellError::ESRCH)?;
            match m.owner {
                None => {
                    m.owner = Some(tid);
                    m.recursive_count = 1;
                    Ok(())
                }
                Some(owner) if owner == tid && m.attr.recursive => {
                    m.recursive_count += 1;
                    Ok(())
                }
                _ => Err(CellError::EBUSY),
            }
        }
        fn mutex_unlock(&mut self, id: u32, tid: u32) -> Result<(), CellError> {
            let m = self.mutexes.get_mut(&id).ok_or(CellError::ESRCH)?;
            match m.owner {
                Some(owner) if owner == tid => {
                    m.recursive_count -= 1;
                    if m.recursive_count == 0 {
                        // Hand off to next waiter if any.
                        if let Some(next) = m.waiters.pop_front() {
                            m.owner = Some(next);
                            m.recursive_count = 1;
                        } else {
                            m.owner = None;
                        }
                    }
                    Ok(())
                }
                _ => Err(CellError::EPERM),
            }
        }

        fn sema_create(
            &mut self,
            attr: SemaAttr,
            initial: i32,
            max: i32,
        ) -> Result<u32, CellError> {
            let id = self.alloc_id();
            self.semas.insert(
                id,
                TestSema { attr, max, value: initial, waiters: VecDeque::new() },
            );
            Ok(id)
        }
        fn sema_destroy(&mut self, id: u32) -> Result<(), CellError> {
            let s = self.semas.get(&id).ok_or(CellError::ESRCH)?;
            if !s.waiters.is_empty() {
                return Err(CellError::EBUSY);
            }
            self.semas.remove(&id);
            Ok(())
        }
        fn sema_post(&mut self, id: u32, count: i32) -> Result<(), CellError> {
            let s = self.semas.get_mut(&id).ok_or(CellError::ESRCH)?;
            if s.value + count > s.max {
                return Err(CellError::EAGAIN);
            }
            s.value += count;
            Ok(())
        }
        fn sema_wait(&mut self, id: u32) -> Result<BlockOutcome, CellError> {
            let s = self.semas.get_mut(&id).ok_or(CellError::ESRCH)?;
            if s.value > 0 {
                s.value -= 1;
                Ok(BlockOutcome::Acquired)
            } else {
                Ok(BlockOutcome::MustBlock)
            }
        }
        fn sema_trywait(&mut self, id: u32) -> Result<(), CellError> {
            let s = self.semas.get_mut(&id).ok_or(CellError::ESRCH)?;
            if s.value > 0 {
                s.value -= 1;
                Ok(())
            } else {
                Err(CellError::EBUSY)
            }
        }
        fn sema_get_value(&self, id: u32) -> Result<i32, CellError> {
            self.semas.get(&id).map(|s| s.value).ok_or(CellError::ESRCH)
        }
    }

    // -- Mutex tests -----------------------------------------------

    #[test]
    fn mutex_create_and_destroy_roundtrip() {
        let mut t = TestTable::default();
        let id = sys_mutex_create(&mut t, MutexAttr::default()).unwrap();
        assert!(id > 0);
        assert_eq!(sys_mutex_destroy(&mut t, id), Ok(()));
    }

    #[test]
    fn mutex_destroy_unknown_is_esrch() {
        let mut t = TestTable::default();
        assert_eq!(sys_mutex_destroy(&mut t, 999), Err(CellError::ESRCH));
    }

    #[test]
    fn mutex_create_rejects_bad_protocol() {
        let mut t = TestTable::default();
        assert_eq!(
            sys_mutex_create(&mut t, MutexAttr { protocol: 0x99, recursive: false }),
            Err(CellError::EINVAL)
        );
    }

    #[test]
    fn mutex_lock_acquires_when_free() {
        let mut t = TestTable::default();
        let id = sys_mutex_create(&mut t, MutexAttr::default()).unwrap();
        assert_eq!(sys_mutex_lock(&mut t, id, 1, TIMEOUT_INFINITE), Ok(BlockOutcome::Acquired));
    }

    #[test]
    fn mutex_lock_blocks_when_held_by_other() {
        let mut t = TestTable::default();
        let id = sys_mutex_create(&mut t, MutexAttr::default()).unwrap();
        sys_mutex_lock(&mut t, id, 1, TIMEOUT_INFINITE).unwrap();
        assert_eq!(
            sys_mutex_lock(&mut t, id, 2, TIMEOUT_INFINITE),
            Ok(BlockOutcome::MustBlock)
        );
    }

    #[test]
    fn mutex_self_lock_non_recursive_is_edeadlk() {
        let mut t = TestTable::default();
        let id = sys_mutex_create(&mut t, MutexAttr::default()).unwrap();
        sys_mutex_lock(&mut t, id, 1, TIMEOUT_INFINITE).unwrap();
        assert_eq!(
            sys_mutex_lock(&mut t, id, 1, TIMEOUT_INFINITE),
            Err(CellError::EDEADLK)
        );
    }

    #[test]
    fn mutex_self_lock_recursive_succeeds() {
        let mut t = TestTable::default();
        let id = sys_mutex_create(
            &mut t,
            MutexAttr { protocol: PROTOCOL_FIFO, recursive: true },
        )
        .unwrap();
        sys_mutex_lock(&mut t, id, 1, TIMEOUT_INFINITE).unwrap();
        assert_eq!(
            sys_mutex_lock(&mut t, id, 1, TIMEOUT_INFINITE),
            Ok(BlockOutcome::Acquired)
        );
    }

    #[test]
    fn mutex_trylock_success_and_failure() {
        let mut t = TestTable::default();
        let id = sys_mutex_create(&mut t, MutexAttr::default()).unwrap();
        assert_eq!(sys_mutex_trylock(&mut t, id, 1), Ok(()));
        // Another tid trying to trylock the held mutex → EBUSY.
        assert_eq!(sys_mutex_trylock(&mut t, id, 2), Err(CellError::EBUSY));
    }

    #[test]
    fn mutex_unlock_not_owner_is_eperm() {
        let mut t = TestTable::default();
        let id = sys_mutex_create(&mut t, MutexAttr::default()).unwrap();
        sys_mutex_lock(&mut t, id, 1, TIMEOUT_INFINITE).unwrap();
        assert_eq!(sys_mutex_unlock(&mut t, id, 2), Err(CellError::EPERM));
    }

    #[test]
    fn mutex_unlock_hands_off_to_waiter() {
        let mut t = TestTable::default();
        let id = sys_mutex_create(&mut t, MutexAttr::default()).unwrap();
        sys_mutex_lock(&mut t, id, 1, TIMEOUT_INFINITE).unwrap();
        sys_mutex_lock(&mut t, id, 2, TIMEOUT_INFINITE).unwrap(); // MustBlock
        sys_mutex_unlock(&mut t, id, 1).unwrap();
        // Waiter 2 should now own the mutex.
        assert_eq!(
            sys_mutex_trylock(&mut t, id, 3),
            Err(CellError::EBUSY) // Because 2 is the owner
        );
    }

    #[test]
    fn mutex_destroy_with_waiters_is_ebusy() {
        let mut t = TestTable::default();
        let id = sys_mutex_create(&mut t, MutexAttr::default()).unwrap();
        sys_mutex_lock(&mut t, id, 1, TIMEOUT_INFINITE).unwrap();
        assert_eq!(sys_mutex_destroy(&mut t, id), Err(CellError::EBUSY));
    }

    // -- Semaphore tests -------------------------------------------

    #[test]
    fn sema_create_rejects_initial_gt_max() {
        let mut t = TestTable::default();
        assert_eq!(
            sys_semaphore_create(&mut t, SemaAttr::default(), 5, 3),
            Err(CellError::EINVAL)
        );
    }

    #[test]
    fn sema_create_rejects_max_zero() {
        let mut t = TestTable::default();
        assert_eq!(
            sys_semaphore_create(&mut t, SemaAttr::default(), 0, 0),
            Err(CellError::EINVAL)
        );
    }

    #[test]
    fn sema_create_rejects_negative_initial() {
        let mut t = TestTable::default();
        assert_eq!(
            sys_semaphore_create(&mut t, SemaAttr::default(), -1, 5),
            Err(CellError::EINVAL)
        );
    }

    #[test]
    fn sema_get_value_returns_initial() {
        let mut t = TestTable::default();
        let id = sys_semaphore_create(&mut t, SemaAttr::default(), 3, 10).unwrap();
        assert_eq!(sys_semaphore_get_value(&t, id), Ok(3));
    }

    #[test]
    fn sema_post_increments_value() {
        let mut t = TestTable::default();
        let id = sys_semaphore_create(&mut t, SemaAttr::default(), 0, 10).unwrap();
        sys_semaphore_post(&mut t, id, 3).unwrap();
        assert_eq!(sys_semaphore_get_value(&t, id), Ok(3));
    }

    #[test]
    fn sema_post_overflow_is_eagain() {
        let mut t = TestTable::default();
        let id = sys_semaphore_create(&mut t, SemaAttr::default(), 0, 5).unwrap();
        assert_eq!(sys_semaphore_post(&mut t, id, 10), Err(CellError::EAGAIN));
    }

    #[test]
    fn sema_post_negative_is_einval() {
        let mut t = TestTable::default();
        let id = sys_semaphore_create(&mut t, SemaAttr::default(), 0, 10).unwrap();
        assert_eq!(sys_semaphore_post(&mut t, id, -1), Err(CellError::EINVAL));
    }

    #[test]
    fn sema_wait_decrements_when_positive() {
        let mut t = TestTable::default();
        let id = sys_semaphore_create(&mut t, SemaAttr::default(), 1, 10).unwrap();
        assert_eq!(
            sys_semaphore_wait(&mut t, id, TIMEOUT_INFINITE),
            Ok(BlockOutcome::Acquired)
        );
        assert_eq!(sys_semaphore_get_value(&t, id), Ok(0));
    }

    #[test]
    fn sema_wait_blocks_when_zero() {
        let mut t = TestTable::default();
        let id = sys_semaphore_create(&mut t, SemaAttr::default(), 0, 10).unwrap();
        assert_eq!(
            sys_semaphore_wait(&mut t, id, TIMEOUT_INFINITE),
            Ok(BlockOutcome::MustBlock)
        );
    }

    #[test]
    fn sema_trywait_on_empty_is_ebusy() {
        let mut t = TestTable::default();
        let id = sys_semaphore_create(&mut t, SemaAttr::default(), 0, 10).unwrap();
        assert_eq!(sys_semaphore_trywait(&mut t, id), Err(CellError::EBUSY));
    }

    #[test]
    fn sema_trywait_on_nonempty_succeeds() {
        let mut t = TestTable::default();
        let id = sys_semaphore_create(&mut t, SemaAttr::default(), 1, 10).unwrap();
        assert_eq!(sys_semaphore_trywait(&mut t, id), Ok(()));
        assert_eq!(sys_semaphore_get_value(&t, id), Ok(0));
    }

    #[test]
    fn sema_destroy_unknown_is_esrch() {
        let mut t = TestTable::default();
        assert_eq!(sys_semaphore_destroy(&mut t, 99), Err(CellError::ESRCH));
    }

    // -- Constants frozen ------------------------------------------

    #[test]
    fn protocol_constants_frozen() {
        assert_eq!(PROTOCOL_FIFO, 0x01);
        assert_eq!(PROTOCOL_PRIORITY, 0x02);
        assert_eq!(PROTOCOL_PRIORITY_INHERIT, 0x03);
    }
}
