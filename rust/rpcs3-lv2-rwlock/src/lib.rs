//! `rpcs3-lv2-rwlock` — reader/writer lock LV2 syscalls.
//!
//! Ports `rpcs3/Emu/Cell/lv2/sys_rwlock.cpp`. An rwlock lets many
//! reader threads hold the lock concurrently, but only one writer at
//! a time (and writers are mutually exclusive with readers). PS3 LV2
//! implements **writer-priority** semantics: as soon as a writer is
//! queued, new readers must block until all current readers drain and
//! the writer has passed through.
//!
//! ## Syscalls covered
//!
//! | LV2 syscall               | Rust wrapper                |
//! |---------------------------|-----------------------------|
//! | `sys_rwlock_create`       | [`sys_rwlock_create`]       |
//! | `sys_rwlock_destroy`      | [`sys_rwlock_destroy`]      |
//! | `sys_rwlock_rlock`        | [`sys_rwlock_rlock`]        |
//! | `sys_rwlock_tryrlock`     | [`sys_rwlock_tryrlock`]     |
//! | `sys_rwlock_runlock`      | [`sys_rwlock_runlock`]      |
//! | `sys_rwlock_wlock`        | [`sys_rwlock_wlock`]        |
//! | `sys_rwlock_trywlock`     | [`sys_rwlock_trywlock`]     |
//! | `sys_rwlock_wunlock`      | [`sys_rwlock_wunlock`]      |
//!
//! ## Semantics (mirroring `sys_rwlock.cpp`)
//!
//! * State is tracked in a single signed atomic `owner`:
//!   - `== 0`: free (no readers, no writer).
//!   - `> 0`: reader count.
//!   - `< 0`: writer thread id (negated) owns exclusively.
//!   For this port we keep readers as an integer count and writer as
//!   an `Option<u32>`, preferring clarity over the C++ bit packing —
//!   the observable behaviour is identical.
//! * `rlock` returns `MustBlock` if a writer owns OR any writer is
//!   queued (writer-priority).
//! * `wlock` returns `MustBlock` if any reader or writer owns.
//! * `runlock` on lock not held by a reader ⇒ `EPERM`.
//! * `wunlock` on lock not held by caller ⇒ `EPERM`.
//! * `destroy` while any thread holds or waits ⇒ `EBUSY`.
//! * `id_base = 0x88000000`.

use rpcs3_emu_types::CellError;

// =====================================================================
// Attribute constants
// =====================================================================

pub const PROTOCOL_FIFO: u32 = 0x01;
pub const PROTOCOL_PRIORITY: u32 = 0x02;
pub const PROTOCOL_PRIORITY_INHERIT: u32 = 0x03;
pub const TIMEOUT_INFINITE: u64 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RwlockAttr {
    pub protocol: u32,
    pub pshared: u32,
    pub flags: i32,
    pub ipc_key: u64,
    pub name: u64,
}

impl Default for RwlockAttr {
    fn default() -> Self {
        Self { protocol: PROTOCOL_FIFO, pshared: 0x200, flags: 0, ipc_key: 0, name: 0 }
    }
}

// =====================================================================
// Outcome for blocking lock attempts
// =====================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockOutcome {
    /// Acquired immediately (read or write, per the caller).
    Acquired,
    /// Caller must park — queued on the appropriate waiter list.
    MustBlock,
    /// `tryXlock` fast path: another owner holds incompatibly.
    Busy,
}

// =====================================================================
// Registry trait
// =====================================================================

pub trait RwlockRegistry {
    fn rwlock_create(&mut self, attr: RwlockAttr) -> Result<u32, CellError>;
    fn rwlock_destroy(&mut self, id: u32) -> Result<(), CellError>;

    /// Attempt a read lock. `tid` is only needed when the caller may
    /// block, so the registry can enqueue the specific thread.
    fn rwlock_rlock(&mut self, id: u32, tid: u32, timeout_us: u64) -> Result<LockOutcome, CellError>;
    fn rwlock_tryrlock(&mut self, id: u32, tid: u32) -> Result<LockOutcome, CellError>;
    fn rwlock_runlock(&mut self, id: u32, tid: u32) -> Result<(), CellError>;

    fn rwlock_wlock(&mut self, id: u32, tid: u32, timeout_us: u64) -> Result<LockOutcome, CellError>;
    fn rwlock_trywlock(&mut self, id: u32, tid: u32) -> Result<LockOutcome, CellError>;
    fn rwlock_wunlock(&mut self, id: u32, tid: u32) -> Result<(), CellError>;
}

// =====================================================================
// Syscalls — thin wrappers doing argument validation
// =====================================================================

fn validate_protocol(protocol: u32) -> Result<(), CellError> {
    match protocol {
        PROTOCOL_FIFO | PROTOCOL_PRIORITY | PROTOCOL_PRIORITY_INHERIT => Ok(()),
        _ => Err(CellError::EINVAL),
    }
}

#[must_use]
pub fn sys_rwlock_create<T: RwlockRegistry + ?Sized>(
    table: &mut T,
    attr: RwlockAttr,
) -> Result<u32, CellError> {
    validate_protocol(attr.protocol)?;
    table.rwlock_create(attr)
}

#[must_use]
pub fn sys_rwlock_destroy<T: RwlockRegistry + ?Sized>(
    table: &mut T,
    id: u32,
) -> Result<(), CellError> {
    table.rwlock_destroy(id)
}

#[must_use]
pub fn sys_rwlock_rlock<T: RwlockRegistry + ?Sized>(
    table: &mut T,
    id: u32,
    tid: u32,
    timeout_us: u64,
) -> Result<LockOutcome, CellError> {
    table.rwlock_rlock(id, tid, timeout_us)
}

#[must_use]
pub fn sys_rwlock_tryrlock<T: RwlockRegistry + ?Sized>(
    table: &mut T,
    id: u32,
    tid: u32,
) -> Result<LockOutcome, CellError> {
    table.rwlock_tryrlock(id, tid)
}

#[must_use]
pub fn sys_rwlock_runlock<T: RwlockRegistry + ?Sized>(
    table: &mut T,
    id: u32,
    tid: u32,
) -> Result<(), CellError> {
    table.rwlock_runlock(id, tid)
}

#[must_use]
pub fn sys_rwlock_wlock<T: RwlockRegistry + ?Sized>(
    table: &mut T,
    id: u32,
    tid: u32,
    timeout_us: u64,
) -> Result<LockOutcome, CellError> {
    table.rwlock_wlock(id, tid, timeout_us)
}

#[must_use]
pub fn sys_rwlock_trywlock<T: RwlockRegistry + ?Sized>(
    table: &mut T,
    id: u32,
    tid: u32,
) -> Result<LockOutcome, CellError> {
    table.rwlock_trywlock(id, tid)
}

#[must_use]
pub fn sys_rwlock_wunlock<T: RwlockRegistry + ?Sized>(
    table: &mut T,
    id: u32,
    tid: u32,
) -> Result<(), CellError> {
    table.rwlock_wunlock(id, tid)
}

// =====================================================================
// Reference implementation
// =====================================================================

#[derive(Debug, Default)]
pub struct TestRwlockRegistry {
    next_id: u32,
    locks: std::collections::BTreeMap<u32, RwlockSlot>,
}

#[derive(Debug, Default)]
#[allow(dead_code)]
struct RwlockSlot {
    attr: RwlockAttr,
    readers: Vec<u32>,    // currently-holding reader tids
    writer: Option<u32>,  // currently-holding writer tid
    read_waiters: Vec<u32>,
    write_waiters: Vec<u32>,
}

impl TestRwlockRegistry {
    fn alloc_id(&mut self) -> u32 {
        self.next_id += 1;
        // Match C++ `lv2_rwlock::id_base = 0x88000000`.
        0x8800_0000 | self.next_id
    }

    /// Test helper: expose waiter counts.
    #[must_use]
    pub fn reader_count(&self, id: u32) -> Option<usize> {
        self.locks.get(&id).map(|s| s.readers.len())
    }
    #[must_use]
    pub fn writer_tid(&self, id: u32) -> Option<u32> {
        self.locks.get(&id).and_then(|s| s.writer)
    }
    #[must_use]
    pub fn write_waiter_count(&self, id: u32) -> Option<usize> {
        self.locks.get(&id).map(|s| s.write_waiters.len())
    }
    #[must_use]
    pub fn read_waiter_count(&self, id: u32) -> Option<usize> {
        self.locks.get(&id).map(|s| s.read_waiters.len())
    }

    /// Simulate scheduler: drain waiters that can now acquire. Returns
    /// the tids that were handed ownership. Call this after a
    /// `runlock` / `wunlock` once the PPU thread is awake.
    pub fn drain_ready(&mut self, id: u32) -> Vec<u32> {
        let Some(slot) = self.locks.get_mut(&id) else { return Vec::new() };

        let mut handed = Vec::new();
        // Writer priority: if a writer is queued and no reader/writer
        // holds, hand to the head of the write queue.
        if slot.writer.is_none() && slot.readers.is_empty() {
            if let Some(w) = slot.write_waiters.first().copied() {
                slot.write_waiters.remove(0);
                slot.writer = Some(w);
                handed.push(w);
                return handed;
            }
            // No writers queued → hand to ALL readers.
            if slot.writer.is_none() {
                let readers: Vec<u32> = slot.read_waiters.drain(..).collect();
                for r in &readers {
                    slot.readers.push(*r);
                }
                handed.extend(readers);
            }
        }
        handed
    }
}

impl RwlockRegistry for TestRwlockRegistry {
    fn rwlock_create(&mut self, attr: RwlockAttr) -> Result<u32, CellError> {
        let id = self.alloc_id();
        self.locks.insert(
            id,
            RwlockSlot { attr, ..RwlockSlot::default() },
        );
        Ok(id)
    }

    fn rwlock_destroy(&mut self, id: u32) -> Result<(), CellError> {
        let slot = self.locks.get(&id).ok_or(CellError::ESRCH)?;
        if !slot.readers.is_empty()
            || slot.writer.is_some()
            || !slot.read_waiters.is_empty()
            || !slot.write_waiters.is_empty()
        {
            return Err(CellError::EBUSY);
        }
        self.locks.remove(&id);
        Ok(())
    }

    fn rwlock_rlock(&mut self, id: u32, tid: u32, _timeout_us: u64) -> Result<LockOutcome, CellError> {
        let slot = self.locks.get_mut(&id).ok_or(CellError::ESRCH)?;
        // Writer priority: block new readers whenever a writer owns OR
        // a writer is queued.
        if slot.writer.is_none() && slot.write_waiters.is_empty() {
            slot.readers.push(tid);
            Ok(LockOutcome::Acquired)
        } else {
            slot.read_waiters.push(tid);
            Ok(LockOutcome::MustBlock)
        }
    }

    fn rwlock_tryrlock(&mut self, id: u32, tid: u32) -> Result<LockOutcome, CellError> {
        let slot = self.locks.get_mut(&id).ok_or(CellError::ESRCH)?;
        if slot.writer.is_none() && slot.write_waiters.is_empty() {
            slot.readers.push(tid);
            Ok(LockOutcome::Acquired)
        } else {
            Ok(LockOutcome::Busy)
        }
    }

    fn rwlock_runlock(&mut self, id: u32, tid: u32) -> Result<(), CellError> {
        let slot = self.locks.get_mut(&id).ok_or(CellError::ESRCH)?;
        let idx = slot
            .readers
            .iter()
            .position(|&t| t == tid)
            .ok_or(CellError::EPERM)?;
        slot.readers.remove(idx);
        Ok(())
    }

    fn rwlock_wlock(&mut self, id: u32, tid: u32, _timeout_us: u64) -> Result<LockOutcome, CellError> {
        let slot = self.locks.get_mut(&id).ok_or(CellError::ESRCH)?;
        if slot.writer == Some(tid) {
            // Writer re-locking its own rwlock is a deadlock per C++.
            return Err(CellError::EDEADLK);
        }
        if slot.readers.is_empty() && slot.writer.is_none() {
            slot.writer = Some(tid);
            Ok(LockOutcome::Acquired)
        } else {
            slot.write_waiters.push(tid);
            Ok(LockOutcome::MustBlock)
        }
    }

    fn rwlock_trywlock(&mut self, id: u32, tid: u32) -> Result<LockOutcome, CellError> {
        let slot = self.locks.get_mut(&id).ok_or(CellError::ESRCH)?;
        if slot.writer == Some(tid) {
            return Err(CellError::EDEADLK);
        }
        if slot.readers.is_empty() && slot.writer.is_none() {
            slot.writer = Some(tid);
            Ok(LockOutcome::Acquired)
        } else {
            Ok(LockOutcome::Busy)
        }
    }

    fn rwlock_wunlock(&mut self, id: u32, tid: u32) -> Result<(), CellError> {
        let slot = self.locks.get_mut(&id).ok_or(CellError::ESRCH)?;
        if slot.writer != Some(tid) {
            return Err(CellError::EPERM);
        }
        slot.writer = None;
        Ok(())
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (TestRwlockRegistry, u32) {
        let mut reg = TestRwlockRegistry::default();
        let id = sys_rwlock_create(&mut reg, RwlockAttr::default()).unwrap();
        (reg, id)
    }

    #[test]
    fn create_rejects_bad_protocol() {
        let mut reg = TestRwlockRegistry::default();
        let bad = RwlockAttr { protocol: 0xDEAD, ..RwlockAttr::default() };
        assert_eq!(sys_rwlock_create(&mut reg, bad).unwrap_err(), CellError::EINVAL);
    }

    #[test]
    fn id_is_in_0x88000000_range() {
        let (_reg, id) = setup();
        assert_eq!(id & 0xFF00_0000, 0x8800_0000);
    }

    #[test]
    fn rlock_acquires_when_free() {
        let (mut reg, id) = setup();
        let out = sys_rwlock_rlock(&mut reg, id, 10, 0).unwrap();
        assert_eq!(out, LockOutcome::Acquired);
        assert_eq!(reg.reader_count(id), Some(1));
    }

    #[test]
    fn multiple_readers_can_acquire_simultaneously() {
        let (mut reg, id) = setup();
        for tid in [1u32, 2, 3, 4] {
            assert_eq!(
                sys_rwlock_rlock(&mut reg, id, tid, 0).unwrap(),
                LockOutcome::Acquired,
            );
        }
        assert_eq!(reg.reader_count(id), Some(4));
    }

    #[test]
    fn wlock_blocks_when_readers_present() {
        let (mut reg, id) = setup();
        sys_rwlock_rlock(&mut reg, id, 1, 0).unwrap();
        let out = sys_rwlock_wlock(&mut reg, id, 99, 0).unwrap();
        assert_eq!(out, LockOutcome::MustBlock);
        assert_eq!(reg.write_waiter_count(id), Some(1));
    }

    #[test]
    fn writer_priority_blocks_new_readers() {
        let (mut reg, id) = setup();
        // Initial reader.
        sys_rwlock_rlock(&mut reg, id, 1, 0).unwrap();
        // Writer queues up.
        sys_rwlock_wlock(&mut reg, id, 99, 0).unwrap();
        // New reader must block even though other readers are active.
        let out = sys_rwlock_rlock(&mut reg, id, 2, 0).unwrap();
        assert_eq!(out, LockOutcome::MustBlock);
        assert_eq!(reg.read_waiter_count(id), Some(1));
    }

    #[test]
    fn wlock_acquires_when_free() {
        let (mut reg, id) = setup();
        assert_eq!(
            sys_rwlock_wlock(&mut reg, id, 5, 0).unwrap(),
            LockOutcome::Acquired,
        );
        assert_eq!(reg.writer_tid(id), Some(5));
    }

    #[test]
    fn rlock_blocks_when_writer_owns() {
        let (mut reg, id) = setup();
        sys_rwlock_wlock(&mut reg, id, 5, 0).unwrap();
        assert_eq!(
            sys_rwlock_rlock(&mut reg, id, 7, 0).unwrap(),
            LockOutcome::MustBlock,
        );
    }

    #[test]
    fn wlock_re_entry_by_same_tid_is_edeadlk() {
        let (mut reg, id) = setup();
        sys_rwlock_wlock(&mut reg, id, 5, 0).unwrap();
        let err = sys_rwlock_wlock(&mut reg, id, 5, 0).unwrap_err();
        assert_eq!(err, CellError::EDEADLK);
    }

    #[test]
    fn runlock_drops_one_reader() {
        let (mut reg, id) = setup();
        sys_rwlock_rlock(&mut reg, id, 1, 0).unwrap();
        sys_rwlock_rlock(&mut reg, id, 2, 0).unwrap();
        sys_rwlock_runlock(&mut reg, id, 1).unwrap();
        assert_eq!(reg.reader_count(id), Some(1));
    }

    #[test]
    fn runlock_by_non_holder_is_eperm() {
        let (mut reg, id) = setup();
        sys_rwlock_rlock(&mut reg, id, 1, 0).unwrap();
        let err = sys_rwlock_runlock(&mut reg, id, 99).unwrap_err();
        assert_eq!(err, CellError::EPERM);
    }

    #[test]
    fn wunlock_by_non_owner_is_eperm() {
        let (mut reg, id) = setup();
        sys_rwlock_wlock(&mut reg, id, 5, 0).unwrap();
        let err = sys_rwlock_wunlock(&mut reg, id, 6).unwrap_err();
        assert_eq!(err, CellError::EPERM);
    }

    #[test]
    fn destroy_while_held_is_ebusy() {
        let (mut reg, id) = setup();
        sys_rwlock_wlock(&mut reg, id, 5, 0).unwrap();
        let err = sys_rwlock_destroy(&mut reg, id).unwrap_err();
        assert_eq!(err, CellError::EBUSY);
    }

    #[test]
    fn destroy_with_waiters_is_ebusy() {
        let (mut reg, id) = setup();
        sys_rwlock_rlock(&mut reg, id, 1, 0).unwrap();
        sys_rwlock_wlock(&mut reg, id, 2, 0).unwrap();
        sys_rwlock_runlock(&mut reg, id, 1).unwrap();
        // Writer still queued (drain_ready not called).
        let err = sys_rwlock_destroy(&mut reg, id).unwrap_err();
        assert_eq!(err, CellError::EBUSY);
    }

    #[test]
    fn destroy_when_free_succeeds() {
        let (mut reg, id) = setup();
        sys_rwlock_destroy(&mut reg, id).unwrap();
        let err = sys_rwlock_destroy(&mut reg, id).unwrap_err();
        assert_eq!(err, CellError::ESRCH);
    }

    #[test]
    fn tryrlock_returns_busy_when_writer_queued() {
        let (mut reg, id) = setup();
        sys_rwlock_rlock(&mut reg, id, 1, 0).unwrap();
        sys_rwlock_wlock(&mut reg, id, 99, 0).unwrap();
        let out = sys_rwlock_tryrlock(&mut reg, id, 2).unwrap();
        assert_eq!(out, LockOutcome::Busy);
    }

    #[test]
    fn trywlock_returns_busy_when_reader_present() {
        let (mut reg, id) = setup();
        sys_rwlock_rlock(&mut reg, id, 1, 0).unwrap();
        let out = sys_rwlock_trywlock(&mut reg, id, 99).unwrap();
        assert_eq!(out, LockOutcome::Busy);
    }

    #[test]
    fn drain_hands_writer_first_then_readers_only_if_no_writer_queued() {
        let (mut reg, id) = setup();
        sys_rwlock_rlock(&mut reg, id, 1, 0).unwrap();
        sys_rwlock_wlock(&mut reg, id, 99, 0).unwrap();   // writer queued
        sys_rwlock_rlock(&mut reg, id, 2, 0).unwrap();    // reader queued
        sys_rwlock_rlock(&mut reg, id, 3, 0).unwrap();    // reader queued

        // Last reader unlocks → writer should be next.
        sys_rwlock_runlock(&mut reg, id, 1).unwrap();
        let handed = reg.drain_ready(id);
        assert_eq!(handed, vec![99]);
        assert_eq!(reg.writer_tid(id), Some(99));
        assert_eq!(reg.read_waiter_count(id), Some(2), "readers still queued");

        // Writer unlocks → both queued readers get handed now.
        sys_rwlock_wunlock(&mut reg, id, 99).unwrap();
        let handed = reg.drain_ready(id);
        assert_eq!(handed, vec![2, 3]);
        assert_eq!(reg.reader_count(id), Some(2));
    }

    #[test]
    fn esrch_on_unknown_id() {
        let mut reg = TestRwlockRegistry::default();
        assert_eq!(sys_rwlock_destroy(&mut reg, 0x8800_0042).unwrap_err(), CellError::ESRCH);
        assert_eq!(sys_rwlock_rlock(&mut reg, 0x8800_0042, 1, 0).unwrap_err(), CellError::ESRCH);
        assert_eq!(sys_rwlock_wlock(&mut reg, 0x8800_0042, 1, 0).unwrap_err(), CellError::ESRCH);
    }

    #[test]
    fn attribute_round_trip_stored() {
        let mut reg = TestRwlockRegistry::default();
        let attr = RwlockAttr { protocol: PROTOCOL_PRIORITY, pshared: 0x100, flags: 0x42, ipc_key: 0x1234, name: 0x5678 };
        let id = sys_rwlock_create(&mut reg, attr).unwrap();
        assert_eq!(reg.locks.get(&id).unwrap().attr, attr);
    }

    #[test]
    fn reader_draining_into_writer_preserves_fifo_order_within_writers() {
        let (mut reg, id) = setup();
        sys_rwlock_rlock(&mut reg, id, 1, 0).unwrap();
        sys_rwlock_wlock(&mut reg, id, 10, 0).unwrap();
        sys_rwlock_wlock(&mut reg, id, 11, 0).unwrap();
        sys_rwlock_runlock(&mut reg, id, 1).unwrap();

        assert_eq!(reg.drain_ready(id), vec![10]);
        sys_rwlock_wunlock(&mut reg, id, 10).unwrap();
        assert_eq!(reg.drain_ready(id), vec![11]);
    }
}
