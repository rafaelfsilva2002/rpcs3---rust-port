//! `rpcs3-hle-cellsync2` — 2nd-generation sync primitives HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellSync2.cpp`. cellSync2 is the
//! replacement for cellSync that the SPURS v2 runtime uses. Unlike
//! cellSync, objects live in user memory with 128-byte alignment and
//! can be shared between PPU threads, PPU fibers, SPURS tasks and
//! SPURS jobs via a `thread_types` bitmask.
//!
//! ## Entry points covered
//!
//! | HLE function                              | Rust wrapper                        |
//! |-------------------------------------------|-------------------------------------|
//! | `cellSync2MutexInitialize`                | [`Sync2::mutex_init`]               |
//! | `cellSync2MutexLock` / `TryLock` / `Unlock` | [`Sync2::mutex_lock`] / …         |
//! | `cellSync2CondInitialize`                 | [`Sync2::cond_init`]                |
//! | `cellSync2CondWait` / `Signal` / `SignalAll` | [`Sync2::cond_wait`] / …         |
//! | `cellSync2SemaphoreInitialize`            | [`Sync2::semaphore_init`]           |
//! | `cellSync2SemaphoreAcquire` / `Release`   | [`Sync2::semaphore_acquire`] / …    |
//! | `cellSync2QueueInitialize`                | [`Sync2::queue_init`]               |
//! | `cellSync2QueuePush` / `Pop` / `GetSize`  | [`Sync2::queue_push`] / …           |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellSync2.h:6-20
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const AGAIN: CellError = CellError(0x8041_0C01);
    pub const INVAL: CellError = CellError(0x8041_0C02);
    pub const NOMEM: CellError = CellError(0x8041_0C04);
    pub const DEADLK: CellError = CellError(0x8041_0C08);
    pub const PERM: CellError = CellError(0x8041_0C09);
    pub const BUSY: CellError = CellError(0x8041_0C0A);
    pub const STAT: CellError = CellError(0x8041_0C0F);
    pub const ALIGN: CellError = CellError(0x8041_0C10);
    pub const NULL_POINTER: CellError = CellError(0x8041_0C11);
    pub const NOT_SUPPORTED_THREAD: CellError = CellError(0x8041_0C12);
    pub const NO_NOTIFIER: CellError = CellError(0x8041_0C13);
    pub const NO_SPU_CONTEXT_STORAGE: CellError = CellError(0x8041_0C14);
}

// =====================================================================
// Constants (cellSync2.h:22-31)
// =====================================================================

pub const NAME_MAX_LENGTH: usize = 31;

pub const THREAD_TYPE_PPU_THREAD: u32 = 1 << 0;
pub const THREAD_TYPE_PPU_FIBER: u32 = 1 << 1;
pub const THREAD_TYPE_SPURS_TASK: u32 = 1 << 2;
pub const THREAD_TYPE_SPURS_JOBQUEUE_JOB: u32 = 1 << 3;
pub const THREAD_TYPE_SPURS_JOB: u32 = 1 << 8;

pub const THREAD_TYPE_ALL_MASK: u32 = THREAD_TYPE_PPU_THREAD
    | THREAD_TYPE_PPU_FIBER
    | THREAD_TYPE_SPURS_TASK
    | THREAD_TYPE_SPURS_JOBQUEUE_JOB
    | THREAD_TYPE_SPURS_JOB;

/// `CellSync2MutexAttribute.padding` / alignment requirement: 128 bytes.
pub const OBJECT_ALIGNMENT: u64 = 128;
/// Each sync2 object is exactly one cache line.
pub const OBJECT_SIZE: usize = 128;
/// `CellSync2*Attribute` structs are also 128 bytes.
pub const ATTRIBUTE_SIZE: usize = 128;

// =====================================================================
// Attribute validation
// =====================================================================

fn validate_name(name: &str) -> Result<(), CellError> {
    // Real lib allows up to NAME_MAX_LENGTH chars + NUL terminator in a
    // 32-byte field.
    if name.len() > NAME_MAX_LENGTH {
        return Err(errors::INVAL);
    }
    if name.bytes().any(|b| b < 0x20 || b == 0x7F) {
        return Err(errors::INVAL);
    }
    Ok(())
}

fn validate_thread_types(thread_types: u32) -> Result<(), CellError> {
    if thread_types == 0 {
        return Err(errors::NOT_SUPPORTED_THREAD);
    }
    if (thread_types & !THREAD_TYPE_ALL_MASK) != 0 {
        return Err(errors::NOT_SUPPORTED_THREAD);
    }
    Ok(())
}

fn validate_alignment(addr: u64) -> Result<(), CellError> {
    if addr == 0 {
        return Err(errors::NULL_POINTER);
    }
    if addr % OBJECT_ALIGNMENT != 0 {
        return Err(errors::ALIGN);
    }
    Ok(())
}

// =====================================================================
// Attribute structs
// =====================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MutexAttribute {
    pub sdk_version: u32,
    pub thread_types: u32,
    pub max_waiters: u16,
    pub recursive: bool,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CondAttribute {
    pub sdk_version: u32,
    pub max_waiters: u16,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemaphoreAttribute {
    pub sdk_version: u32,
    pub thread_types: u32,
    pub max_waiters: u16,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueAttribute {
    pub sdk_version: u32,
    pub thread_types: u32,
    pub element_size: u32,
    pub depth: u32,
    pub max_push_waiters: u16,
    pub max_pop_waiters: u16,
    pub name: String,
}

// =====================================================================
// Internal state
// =====================================================================

#[derive(Clone, Debug)]
struct Mutex {
    id: u32,
    #[allow(dead_code)]
    thread_types: u32,
    max_waiters: u16,
    recursive: bool,
    #[allow(dead_code)]
    name: String,
    owner: Option<u64>, // caller id (thread/fiber token)
    recursion_count: u32,
    waiters: u16,
}

#[derive(Clone, Debug)]
struct Cond {
    id: u32,
    mutex_id: u32,
    max_waiters: u16,
    #[allow(dead_code)]
    name: String,
    waiters: u16,
}

#[derive(Clone, Debug)]
struct Semaphore {
    id: u32,
    #[allow(dead_code)]
    thread_types: u32,
    max_waiters: u16,
    #[allow(dead_code)]
    name: String,
    count: i32,
    #[allow(dead_code)]
    initial_count: i32,
    max_count: i32,
    waiters: u16,
}

#[derive(Clone, Debug)]
struct Queue {
    id: u32,
    #[allow(dead_code)]
    thread_types: u32,
    element_size: u32,
    depth: u32,
    max_push_waiters: u16,
    max_pop_waiters: u16,
    #[allow(dead_code)]
    name: String,
    elements: std::collections::VecDeque<Vec<u8>>,
    push_waiters: u16,
    pop_waiters: u16,
}

// =====================================================================
// Manager
// =====================================================================

#[derive(Clone, Debug, Default)]
pub struct Sync2 {
    mutexes: Vec<Mutex>,
    conds: Vec<Cond>,
    semaphores: Vec<Semaphore>,
    queues: Vec<Queue>,
    next_id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockOutcome {
    Acquired,
    WouldBlock,
}

impl Sync2 {
    #[must_use]
    pub fn new() -> Self {
        Self { next_id: 1, ..Default::default() }
    }

    fn next_id(&mut self) -> Result<u32, CellError> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or(errors::NOMEM)?;
        Ok(id)
    }

    // ----------------- Mutex -----------------

    pub fn mutex_init(&mut self, addr: u64, attr: &MutexAttribute) -> Result<u32, CellError> {
        validate_alignment(addr)?;
        validate_thread_types(attr.thread_types)?;
        validate_name(&attr.name)?;
        let id = self.next_id()?;
        self.mutexes.push(Mutex {
            id,
            thread_types: attr.thread_types,
            max_waiters: attr.max_waiters,
            recursive: attr.recursive,
            name: attr.name.clone(),
            owner: None,
            recursion_count: 0,
            waiters: 0,
        });
        Ok(id)
    }

    pub fn mutex_finalize(&mut self, id: u32) -> Result<(), CellError> {
        let idx = self.mutexes.iter().position(|m| m.id == id).ok_or(errors::STAT)?;
        if self.mutexes[idx].owner.is_some() {
            return Err(errors::BUSY);
        }
        if self.mutexes[idx].waiters > 0 {
            return Err(errors::BUSY);
        }
        // Any cond that still references this mutex?
        if self.conds.iter().any(|c| c.mutex_id == id) {
            return Err(errors::BUSY);
        }
        self.mutexes.remove(idx);
        Ok(())
    }

    pub fn mutex_try_lock(&mut self, id: u32, caller: u64) -> Result<LockOutcome, CellError> {
        let m = self.mutex_mut(id)?;
        if caller == 0 {
            return Err(errors::NULL_POINTER);
        }
        match m.owner {
            None => {
                m.owner = Some(caller);
                m.recursion_count = 1;
                Ok(LockOutcome::Acquired)
            }
            Some(o) if o == caller && m.recursive => {
                m.recursion_count = m.recursion_count.checked_add(1).ok_or(errors::STAT)?;
                Ok(LockOutcome::Acquired)
            }
            Some(o) if o == caller => Err(errors::DEADLK),
            Some(_) => Ok(LockOutcome::WouldBlock),
        }
    }

    pub fn mutex_lock(&mut self, id: u32, caller: u64) -> Result<LockOutcome, CellError> {
        match self.mutex_try_lock(id, caller)? {
            LockOutcome::Acquired => Ok(LockOutcome::Acquired),
            LockOutcome::WouldBlock => {
                let m = self.mutex_mut(id)?;
                if m.max_waiters > 0 && m.waiters >= m.max_waiters {
                    return Err(errors::AGAIN);
                }
                m.waiters = m.waiters.saturating_add(1);
                Ok(LockOutcome::WouldBlock)
            }
        }
    }

    pub fn mutex_unlock(&mut self, id: u32, caller: u64) -> Result<(), CellError> {
        let m = self.mutex_mut(id)?;
        let owner = m.owner.ok_or(errors::PERM)?;
        if owner != caller {
            return Err(errors::PERM);
        }
        if m.recursion_count > 1 {
            m.recursion_count -= 1;
            return Ok(());
        }
        m.owner = None;
        m.recursion_count = 0;
        Ok(())
    }

    // ----------------- Cond -----------------

    pub fn cond_init(&mut self, addr: u64, mutex_id: u32, attr: &CondAttribute) -> Result<u32, CellError> {
        validate_alignment(addr)?;
        validate_name(&attr.name)?;
        // The mutex must exist — sync2 binds cond to an existing mutex.
        if !self.mutexes.iter().any(|m| m.id == mutex_id) {
            return Err(errors::INVAL);
        }
        let id = self.next_id()?;
        self.conds.push(Cond {
            id,
            mutex_id,
            max_waiters: attr.max_waiters,
            name: attr.name.clone(),
            waiters: 0,
        });
        Ok(id)
    }

    pub fn cond_finalize(&mut self, id: u32) -> Result<(), CellError> {
        let idx = self.conds.iter().position(|c| c.id == id).ok_or(errors::STAT)?;
        if self.conds[idx].waiters > 0 {
            return Err(errors::BUSY);
        }
        self.conds.remove(idx);
        Ok(())
    }

    /// Wait implicitly releases the mutex; return value tells caller if
    /// a block happened (test-only hook — real lib parks the thread).
    pub fn cond_wait(&mut self, id: u32, caller: u64) -> Result<(), CellError> {
        let cond_idx = self.conds.iter().position(|c| c.id == id).ok_or(errors::STAT)?;
        let mutex_id = self.conds[cond_idx].mutex_id;
        // Caller must own the mutex.
        let m = self.mutex_mut(mutex_id)?;
        if m.owner != Some(caller) {
            return Err(errors::PERM);
        }
        if self.conds[cond_idx].max_waiters > 0
            && self.conds[cond_idx].waiters >= self.conds[cond_idx].max_waiters
        {
            return Err(errors::AGAIN);
        }
        // Release the mutex and register as waiter.
        let m = self.mutex_mut(mutex_id)?;
        m.owner = None;
        m.recursion_count = 0;
        self.conds[cond_idx].waiters = self.conds[cond_idx].waiters.saturating_add(1);
        Ok(())
    }

    pub fn cond_signal(&mut self, id: u32) -> Result<u32, CellError> {
        let cond_idx = self.conds.iter().position(|c| c.id == id).ok_or(errors::STAT)?;
        if self.conds[cond_idx].waiters == 0 {
            return Ok(0);
        }
        self.conds[cond_idx].waiters -= 1;
        Ok(1)
    }

    pub fn cond_signal_all(&mut self, id: u32) -> Result<u32, CellError> {
        let cond_idx = self.conds.iter().position(|c| c.id == id).ok_or(errors::STAT)?;
        let n = self.conds[cond_idx].waiters;
        self.conds[cond_idx].waiters = 0;
        Ok(u32::from(n))
    }

    // ----------------- Semaphore -----------------

    pub fn semaphore_init(
        &mut self,
        addr: u64,
        initial: i32,
        max: i32,
        attr: &SemaphoreAttribute,
    ) -> Result<u32, CellError> {
        validate_alignment(addr)?;
        validate_thread_types(attr.thread_types)?;
        validate_name(&attr.name)?;
        if max <= 0 || initial < 0 || initial > max {
            return Err(errors::INVAL);
        }
        let id = self.next_id()?;
        self.semaphores.push(Semaphore {
            id,
            thread_types: attr.thread_types,
            max_waiters: attr.max_waiters,
            name: attr.name.clone(),
            count: initial,
            initial_count: initial,
            max_count: max,
            waiters: 0,
        });
        Ok(id)
    }

    pub fn semaphore_finalize(&mut self, id: u32) -> Result<(), CellError> {
        let idx = self.semaphores.iter().position(|s| s.id == id).ok_or(errors::STAT)?;
        if self.semaphores[idx].waiters > 0 {
            return Err(errors::BUSY);
        }
        self.semaphores.remove(idx);
        Ok(())
    }

    pub fn semaphore_try_acquire(&mut self, id: u32) -> Result<LockOutcome, CellError> {
        let s = self.sem_mut(id)?;
        if s.count > 0 {
            s.count -= 1;
            Ok(LockOutcome::Acquired)
        } else {
            Ok(LockOutcome::WouldBlock)
        }
    }

    pub fn semaphore_acquire(&mut self, id: u32) -> Result<LockOutcome, CellError> {
        match self.semaphore_try_acquire(id)? {
            LockOutcome::Acquired => Ok(LockOutcome::Acquired),
            LockOutcome::WouldBlock => {
                let s = self.sem_mut(id)?;
                if s.max_waiters > 0 && s.waiters >= s.max_waiters {
                    return Err(errors::AGAIN);
                }
                s.waiters = s.waiters.saturating_add(1);
                Ok(LockOutcome::WouldBlock)
            }
        }
    }

    pub fn semaphore_release(&mut self, id: u32, count: i32) -> Result<(), CellError> {
        if count <= 0 {
            return Err(errors::INVAL);
        }
        let s = self.sem_mut(id)?;
        let new_count = s.count.checked_add(count).ok_or(errors::INVAL)?;
        if new_count > s.max_count {
            return Err(errors::INVAL);
        }
        s.count = new_count;
        // Wake up to `count` waiters.
        let to_wake = s.waiters.min(count as u16);
        s.waiters -= to_wake;
        Ok(())
    }

    pub fn semaphore_count(&self, id: u32) -> Result<i32, CellError> {
        let idx = self.semaphores.iter().position(|s| s.id == id).ok_or(errors::STAT)?;
        Ok(self.semaphores[idx].count)
    }

    // ----------------- Queue -----------------

    pub fn queue_init(&mut self, addr: u64, attr: &QueueAttribute) -> Result<u32, CellError> {
        validate_alignment(addr)?;
        validate_thread_types(attr.thread_types)?;
        validate_name(&attr.name)?;
        if attr.element_size == 0 || attr.depth == 0 {
            return Err(errors::INVAL);
        }
        if attr.element_size > 16 * 1024 || attr.depth > 16 * 1024 {
            return Err(errors::NOMEM);
        }
        let id = self.next_id()?;
        self.queues.push(Queue {
            id,
            thread_types: attr.thread_types,
            element_size: attr.element_size,
            depth: attr.depth,
            max_push_waiters: attr.max_push_waiters,
            max_pop_waiters: attr.max_pop_waiters,
            name: attr.name.clone(),
            elements: std::collections::VecDeque::with_capacity(attr.depth as usize),
            push_waiters: 0,
            pop_waiters: 0,
        });
        Ok(id)
    }

    pub fn queue_finalize(&mut self, id: u32) -> Result<(), CellError> {
        let idx = self.queues.iter().position(|q| q.id == id).ok_or(errors::STAT)?;
        if self.queues[idx].push_waiters > 0 || self.queues[idx].pop_waiters > 0 {
            return Err(errors::BUSY);
        }
        self.queues.remove(idx);
        Ok(())
    }

    pub fn queue_try_push(&mut self, id: u32, data: &[u8]) -> Result<LockOutcome, CellError> {
        let q = self.queue_mut(id)?;
        if data.len() as u32 != q.element_size {
            return Err(errors::INVAL);
        }
        if q.elements.len() as u32 >= q.depth {
            return Ok(LockOutcome::WouldBlock);
        }
        q.elements.push_back(data.to_vec());
        Ok(LockOutcome::Acquired)
    }

    pub fn queue_push(&mut self, id: u32, data: &[u8]) -> Result<LockOutcome, CellError> {
        match self.queue_try_push(id, data)? {
            LockOutcome::Acquired => Ok(LockOutcome::Acquired),
            LockOutcome::WouldBlock => {
                let q = self.queue_mut(id)?;
                if q.max_push_waiters > 0 && q.push_waiters >= q.max_push_waiters {
                    return Err(errors::AGAIN);
                }
                q.push_waiters = q.push_waiters.saturating_add(1);
                Ok(LockOutcome::WouldBlock)
            }
        }
    }

    pub fn queue_try_pop(&mut self, id: u32, out: &mut [u8]) -> Result<LockOutcome, CellError> {
        let q = self.queue_mut(id)?;
        if out.len() as u32 != q.element_size {
            return Err(errors::INVAL);
        }
        if let Some(data) = q.elements.pop_front() {
            out.copy_from_slice(&data);
            Ok(LockOutcome::Acquired)
        } else {
            Ok(LockOutcome::WouldBlock)
        }
    }

    pub fn queue_pop(&mut self, id: u32, out: &mut [u8]) -> Result<LockOutcome, CellError> {
        match self.queue_try_pop(id, out)? {
            LockOutcome::Acquired => Ok(LockOutcome::Acquired),
            LockOutcome::WouldBlock => {
                let q = self.queue_mut(id)?;
                if q.max_pop_waiters > 0 && q.pop_waiters >= q.max_pop_waiters {
                    return Err(errors::AGAIN);
                }
                q.pop_waiters = q.pop_waiters.saturating_add(1);
                Ok(LockOutcome::WouldBlock)
            }
        }
    }

    pub fn queue_size(&self, id: u32) -> Result<u32, CellError> {
        let idx = self.queues.iter().position(|q| q.id == id).ok_or(errors::STAT)?;
        Ok(self.queues[idx].elements.len() as u32)
    }

    pub fn queue_depth(&self, id: u32) -> Result<u32, CellError> {
        let idx = self.queues.iter().position(|q| q.id == id).ok_or(errors::STAT)?;
        Ok(self.queues[idx].depth)
    }

    // ----------------- Helpers -----------------

    fn mutex_mut(&mut self, id: u32) -> Result<&mut Mutex, CellError> {
        self.mutexes.iter_mut().find(|m| m.id == id).ok_or(errors::STAT)
    }

    fn sem_mut(&mut self, id: u32) -> Result<&mut Semaphore, CellError> {
        self.semaphores.iter_mut().find(|s| s.id == id).ok_or(errors::STAT)
    }

    fn queue_mut(&mut self, id: u32) -> Result<&mut Queue, CellError> {
        self.queues.iter_mut().find(|q| q.id == id).ok_or(errors::STAT)
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const ADDR: u64 = 0x1000_0080;

    fn mutex_attr(recursive: bool) -> MutexAttribute {
        MutexAttribute {
            sdk_version: 0x370_0000,
            thread_types: THREAD_TYPE_PPU_THREAD,
            max_waiters: 4,
            recursive,
            name: "m".into(),
        }
    }

    fn cond_attr() -> CondAttribute {
        CondAttribute { sdk_version: 0x370_0000, max_waiters: 4, name: "c".into() }
    }

    fn sem_attr() -> SemaphoreAttribute {
        SemaphoreAttribute {
            sdk_version: 0x370_0000,
            thread_types: THREAD_TYPE_PPU_THREAD,
            max_waiters: 4,
            name: "s".into(),
        }
    }

    fn queue_attr(element_size: u32, depth: u32) -> QueueAttribute {
        QueueAttribute {
            sdk_version: 0x370_0000,
            thread_types: THREAD_TYPE_PPU_THREAD,
            element_size,
            depth,
            max_push_waiters: 2,
            max_pop_waiters: 2,
            name: "q".into(),
        }
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::AGAIN.0, 0x8041_0C01);
        assert_eq!(errors::INVAL.0, 0x8041_0C02);
        assert_eq!(errors::NOMEM.0, 0x8041_0C04);
        assert_eq!(errors::DEADLK.0, 0x8041_0C08);
        assert_eq!(errors::PERM.0, 0x8041_0C09);
        assert_eq!(errors::BUSY.0, 0x8041_0C0A);
        assert_eq!(errors::STAT.0, 0x8041_0C0F);
        assert_eq!(errors::ALIGN.0, 0x8041_0C10);
        assert_eq!(errors::NULL_POINTER.0, 0x8041_0C11);
        assert_eq!(errors::NOT_SUPPORTED_THREAD.0, 0x8041_0C12);
        assert_eq!(errors::NO_NOTIFIER.0, 0x8041_0C13);
        assert_eq!(errors::NO_SPU_CONTEXT_STORAGE.0, 0x8041_0C14);
    }

    #[test]
    fn constants_stable() {
        assert_eq!(NAME_MAX_LENGTH, 31);
        assert_eq!(THREAD_TYPE_PPU_THREAD, 1);
        assert_eq!(THREAD_TYPE_PPU_FIBER, 2);
        assert_eq!(THREAD_TYPE_SPURS_TASK, 4);
        assert_eq!(THREAD_TYPE_SPURS_JOBQUEUE_JOB, 8);
        assert_eq!(THREAD_TYPE_SPURS_JOB, 0x100);
        assert_eq!(OBJECT_SIZE, 128);
        assert_eq!(OBJECT_ALIGNMENT, 128);
        assert_eq!(ATTRIBUTE_SIZE, 128);
    }

    #[test]
    fn mutex_init_alignment_rejected() {
        let mut s = Sync2::new();
        assert_eq!(s.mutex_init(ADDR + 1, &mutex_attr(false)), Err(errors::ALIGN));
    }

    #[test]
    fn mutex_init_null_addr_rejected() {
        let mut s = Sync2::new();
        assert_eq!(s.mutex_init(0, &mutex_attr(false)), Err(errors::NULL_POINTER));
    }

    #[test]
    fn mutex_init_bad_thread_types_rejected() {
        let mut s = Sync2::new();
        let mut a = mutex_attr(false);
        a.thread_types = 0;
        assert_eq!(s.mutex_init(ADDR, &a), Err(errors::NOT_SUPPORTED_THREAD));
        a.thread_types = 0x8000_0000;
        assert_eq!(s.mutex_init(ADDR, &a), Err(errors::NOT_SUPPORTED_THREAD));
    }

    #[test]
    fn mutex_init_long_name_rejected() {
        let mut s = Sync2::new();
        let mut a = mutex_attr(false);
        a.name = "x".repeat(NAME_MAX_LENGTH + 1);
        assert_eq!(s.mutex_init(ADDR, &a), Err(errors::INVAL));
    }

    #[test]
    fn mutex_init_control_char_name_rejected() {
        let mut s = Sync2::new();
        let mut a = mutex_attr(false);
        a.name = "foo\tbar".into();
        assert_eq!(s.mutex_init(ADDR, &a), Err(errors::INVAL));
    }

    #[test]
    fn mutex_lock_unlock_round_trip() {
        let mut s = Sync2::new();
        let m = s.mutex_init(ADDR, &mutex_attr(false)).unwrap();
        assert_eq!(s.mutex_lock(m, 1), Ok(LockOutcome::Acquired));
        s.mutex_unlock(m, 1).unwrap();
    }

    #[test]
    fn mutex_try_lock_would_block_on_contention() {
        let mut s = Sync2::new();
        let m = s.mutex_init(ADDR, &mutex_attr(false)).unwrap();
        s.mutex_lock(m, 1).unwrap();
        assert_eq!(s.mutex_try_lock(m, 2), Ok(LockOutcome::WouldBlock));
    }

    #[test]
    fn mutex_null_caller_rejected() {
        let mut s = Sync2::new();
        let m = s.mutex_init(ADDR, &mutex_attr(false)).unwrap();
        assert_eq!(s.mutex_try_lock(m, 0), Err(errors::NULL_POINTER));
    }

    #[test]
    fn mutex_non_recursive_self_relock_is_deadlock() {
        let mut s = Sync2::new();
        let m = s.mutex_init(ADDR, &mutex_attr(false)).unwrap();
        s.mutex_lock(m, 1).unwrap();
        assert_eq!(s.mutex_try_lock(m, 1), Err(errors::DEADLK));
    }

    #[test]
    fn mutex_recursive_self_relock_ok() {
        let mut s = Sync2::new();
        let m = s.mutex_init(ADDR, &mutex_attr(true)).unwrap();
        s.mutex_lock(m, 1).unwrap();
        s.mutex_lock(m, 1).unwrap();
        s.mutex_unlock(m, 1).unwrap();
        s.mutex_unlock(m, 1).unwrap();
    }

    #[test]
    fn mutex_unlock_not_owner_is_perm() {
        let mut s = Sync2::new();
        let m = s.mutex_init(ADDR, &mutex_attr(false)).unwrap();
        s.mutex_lock(m, 1).unwrap();
        assert_eq!(s.mutex_unlock(m, 2), Err(errors::PERM));
    }

    #[test]
    fn mutex_unlock_unowned_is_perm() {
        let mut s = Sync2::new();
        let m = s.mutex_init(ADDR, &mutex_attr(false)).unwrap();
        assert_eq!(s.mutex_unlock(m, 1), Err(errors::PERM));
    }

    #[test]
    fn mutex_lock_exhausts_max_waiters_again() {
        let mut s = Sync2::new();
        let mut a = mutex_attr(false);
        a.max_waiters = 1;
        let m = s.mutex_init(ADDR, &a).unwrap();
        s.mutex_lock(m, 1).unwrap();
        assert_eq!(s.mutex_lock(m, 2), Ok(LockOutcome::WouldBlock));
        assert_eq!(s.mutex_lock(m, 3), Err(errors::AGAIN));
    }

    #[test]
    fn mutex_finalize_held_is_busy() {
        let mut s = Sync2::new();
        let m = s.mutex_init(ADDR, &mutex_attr(false)).unwrap();
        s.mutex_lock(m, 1).unwrap();
        assert_eq!(s.mutex_finalize(m), Err(errors::BUSY));
    }

    #[test]
    fn mutex_finalize_bad_id_rejected() {
        let mut s = Sync2::new();
        assert_eq!(s.mutex_finalize(999), Err(errors::STAT));
    }

    #[test]
    fn cond_init_requires_valid_mutex() {
        let mut s = Sync2::new();
        assert_eq!(s.cond_init(ADDR, 999, &cond_attr()), Err(errors::INVAL));
    }

    #[test]
    fn cond_wait_requires_mutex_ownership() {
        let mut s = Sync2::new();
        let m = s.mutex_init(ADDR, &mutex_attr(false)).unwrap();
        let c = s.cond_init(ADDR, m, &cond_attr()).unwrap();
        assert_eq!(s.cond_wait(c, 1), Err(errors::PERM));
    }

    #[test]
    fn cond_wait_releases_mutex() {
        let mut s = Sync2::new();
        let m = s.mutex_init(ADDR, &mutex_attr(false)).unwrap();
        let c = s.cond_init(ADDR, m, &cond_attr()).unwrap();
        s.mutex_lock(m, 1).unwrap();
        s.cond_wait(c, 1).unwrap();
        // After wait, mutex should be available to another caller.
        assert_eq!(s.mutex_try_lock(m, 2), Ok(LockOutcome::Acquired));
    }

    #[test]
    fn cond_signal_no_waiters_returns_zero() {
        let mut s = Sync2::new();
        let m = s.mutex_init(ADDR, &mutex_attr(false)).unwrap();
        let c = s.cond_init(ADDR, m, &cond_attr()).unwrap();
        assert_eq!(s.cond_signal(c), Ok(0));
    }

    #[test]
    fn cond_signal_one_waiter() {
        let mut s = Sync2::new();
        let m = s.mutex_init(ADDR, &mutex_attr(false)).unwrap();
        let c = s.cond_init(ADDR, m, &cond_attr()).unwrap();
        s.mutex_lock(m, 1).unwrap();
        s.cond_wait(c, 1).unwrap();
        s.mutex_lock(m, 2).unwrap();
        s.cond_wait(c, 2).unwrap();
        assert_eq!(s.cond_signal(c), Ok(1));
        assert_eq!(s.cond_signal_all(c), Ok(1));
    }

    #[test]
    fn cond_finalize_with_waiters_is_busy() {
        let mut s = Sync2::new();
        let m = s.mutex_init(ADDR, &mutex_attr(false)).unwrap();
        let c = s.cond_init(ADDR, m, &cond_attr()).unwrap();
        s.mutex_lock(m, 1).unwrap();
        s.cond_wait(c, 1).unwrap();
        assert_eq!(s.cond_finalize(c), Err(errors::BUSY));
    }

    #[test]
    fn mutex_finalize_with_bound_cond_is_busy() {
        let mut s = Sync2::new();
        let m = s.mutex_init(ADDR, &mutex_attr(false)).unwrap();
        let _c = s.cond_init(ADDR, m, &cond_attr()).unwrap();
        assert_eq!(s.mutex_finalize(m), Err(errors::BUSY));
    }

    #[test]
    fn semaphore_init_invalid_counts_rejected() {
        let mut s = Sync2::new();
        assert_eq!(s.semaphore_init(ADDR, -1, 5, &sem_attr()), Err(errors::INVAL));
        assert_eq!(s.semaphore_init(ADDR, 6, 5, &sem_attr()), Err(errors::INVAL));
        assert_eq!(s.semaphore_init(ADDR, 0, 0, &sem_attr()), Err(errors::INVAL));
    }

    #[test]
    fn semaphore_acquire_decrements_count() {
        let mut s = Sync2::new();
        let sem = s.semaphore_init(ADDR, 2, 5, &sem_attr()).unwrap();
        assert_eq!(s.semaphore_count(sem), Ok(2));
        s.semaphore_acquire(sem).unwrap();
        assert_eq!(s.semaphore_count(sem), Ok(1));
        s.semaphore_acquire(sem).unwrap();
        assert_eq!(s.semaphore_count(sem), Ok(0));
        assert_eq!(s.semaphore_acquire(sem), Ok(LockOutcome::WouldBlock));
    }

    #[test]
    fn semaphore_release_increments_count() {
        let mut s = Sync2::new();
        let sem = s.semaphore_init(ADDR, 0, 5, &sem_attr()).unwrap();
        s.semaphore_release(sem, 3).unwrap();
        assert_eq!(s.semaphore_count(sem), Ok(3));
    }

    #[test]
    fn semaphore_release_over_max_rejected() {
        let mut s = Sync2::new();
        let sem = s.semaphore_init(ADDR, 4, 5, &sem_attr()).unwrap();
        assert_eq!(s.semaphore_release(sem, 2), Err(errors::INVAL));
    }

    #[test]
    fn semaphore_release_non_positive_rejected() {
        let mut s = Sync2::new();
        let sem = s.semaphore_init(ADDR, 0, 5, &sem_attr()).unwrap();
        assert_eq!(s.semaphore_release(sem, 0), Err(errors::INVAL));
        assert_eq!(s.semaphore_release(sem, -1), Err(errors::INVAL));
    }

    #[test]
    fn queue_init_zero_element_or_depth_rejected() {
        let mut s = Sync2::new();
        assert_eq!(s.queue_init(ADDR, &queue_attr(0, 4)), Err(errors::INVAL));
        assert_eq!(s.queue_init(ADDR, &queue_attr(4, 0)), Err(errors::INVAL));
    }

    #[test]
    fn queue_init_huge_size_rejected() {
        let mut s = Sync2::new();
        assert_eq!(s.queue_init(ADDR, &queue_attr(32 * 1024, 4)), Err(errors::NOMEM));
    }

    #[test]
    fn queue_push_pop_round_trip() {
        let mut s = Sync2::new();
        let q = s.queue_init(ADDR, &queue_attr(4, 3)).unwrap();
        assert_eq!(s.queue_push(q, &[1, 2, 3, 4]), Ok(LockOutcome::Acquired));
        assert_eq!(s.queue_push(q, &[5, 6, 7, 8]), Ok(LockOutcome::Acquired));
        assert_eq!(s.queue_size(q), Ok(2));
        let mut buf = [0u8; 4];
        s.queue_pop(q, &mut buf).unwrap();
        assert_eq!(buf, [1, 2, 3, 4]);
        s.queue_pop(q, &mut buf).unwrap();
        assert_eq!(buf, [5, 6, 7, 8]);
    }

    #[test]
    fn queue_push_mismatched_element_size_rejected() {
        let mut s = Sync2::new();
        let q = s.queue_init(ADDR, &queue_attr(4, 3)).unwrap();
        assert_eq!(s.queue_push(q, &[1, 2]), Err(errors::INVAL));
    }

    #[test]
    fn queue_push_full_would_block() {
        let mut s = Sync2::new();
        let q = s.queue_init(ADDR, &queue_attr(1, 1)).unwrap();
        s.queue_push(q, &[1]).unwrap();
        assert_eq!(s.queue_push(q, &[2]), Ok(LockOutcome::WouldBlock));
    }

    #[test]
    fn queue_pop_empty_would_block() {
        let mut s = Sync2::new();
        let q = s.queue_init(ADDR, &queue_attr(1, 1)).unwrap();
        let mut buf = [0u8; 1];
        assert_eq!(s.queue_pop(q, &mut buf), Ok(LockOutcome::WouldBlock));
    }

    #[test]
    fn queue_push_exhausts_max_waiters_again() {
        let mut s = Sync2::new();
        let mut a = queue_attr(1, 1);
        a.max_push_waiters = 1;
        let q = s.queue_init(ADDR, &a).unwrap();
        s.queue_push(q, &[1]).unwrap();
        assert_eq!(s.queue_push(q, &[2]), Ok(LockOutcome::WouldBlock));
        assert_eq!(s.queue_push(q, &[3]), Err(errors::AGAIN));
    }

    #[test]
    fn queue_depth_and_size_queryable() {
        let mut s = Sync2::new();
        let q = s.queue_init(ADDR, &queue_attr(2, 5)).unwrap();
        assert_eq!(s.queue_depth(q), Ok(5));
        assert_eq!(s.queue_size(q), Ok(0));
        s.queue_push(q, &[1, 2]).unwrap();
        assert_eq!(s.queue_size(q), Ok(1));
    }

    #[test]
    fn full_sync2_lifecycle_smoke() {
        let mut s = Sync2::new();
        let m = s.mutex_init(ADDR, &mutex_attr(true)).unwrap();
        let c = s.cond_init(ADDR + OBJECT_SIZE as u64, m, &cond_attr()).unwrap();
        let sem = s.semaphore_init(ADDR + 2 * OBJECT_SIZE as u64, 1, 4, &sem_attr()).unwrap();
        let q = s.queue_init(ADDR + 3 * OBJECT_SIZE as u64, &queue_attr(4, 2)).unwrap();
        s.mutex_lock(m, 42).unwrap();
        s.mutex_lock(m, 42).unwrap();
        s.mutex_unlock(m, 42).unwrap();
        s.cond_wait(c, 42).unwrap();
        s.cond_signal(c).unwrap();
        s.semaphore_acquire(sem).unwrap();
        s.semaphore_release(sem, 1).unwrap();
        s.queue_push(q, &[0xA, 0xB, 0xC, 0xD]).unwrap();
        let mut buf = [0u8; 4];
        s.queue_pop(q, &mut buf).unwrap();
        assert_eq!(buf, [0xA, 0xB, 0xC, 0xD]);
        // Finalize in dependency order.
        s.queue_finalize(q).unwrap();
        s.semaphore_finalize(sem).unwrap();
        s.cond_finalize(c).unwrap();
        s.mutex_finalize(m).unwrap();
    }
}
