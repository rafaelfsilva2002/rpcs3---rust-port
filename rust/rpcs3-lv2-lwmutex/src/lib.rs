//! `rpcs3-lv2-lwmutex` — lightweight mutex LV2 syscalls.
//!
//! Ports `rpcs3/Emu/Cell/lv2/sys_lwmutex.cpp`. Unlike the regular
//! [`sys_mutex`](../../rpcs3-lv2-sync/index.html) family, **lwmutex
//! keeps its state in a user-memory control struct** (32 bytes,
//! big-endian layout): the kernel only sees a handle + the pointer to
//! the struct and cooperates with the user side via atomic ops on the
//! `owner`/`waiter` fields. The kernel side keeps a queue of parked
//! threads in a separate object, but the "locked / held by tid X"
//! state is the user-memory control word.
//!
//! This port models the control struct as [`LwMutexControl`] (a
//! `#[repr(C)]` 32-byte BE layout — byte-exact vs. `sys_lwmutex_t` in
//! C++) and exposes free functions on it for the user-side fast path.
//! The kernel-side waiter queue is behind the [`LwMutexTable`] trait.
//!
//! ## Sentinel values (frozen, from `sys_lwmutex.h`)
//!
//! | Const              | Value        | Meaning                    |
//! |--------------------|--------------|----------------------------|
//! | `LWMUTEX_FREE`     | `0xFFFFFFFF` | No owner                   |
//! | `LWMUTEX_DEAD`     | `0xFFFFFFFE` | Destroyed (poisoned)       |
//! | `LWMUTEX_RESERVED` | `0xFFFFFFFD` | Temporarily reserved for handoff |
//!
//! ## Syscalls covered
//!
//! | LV2 syscall             | Rust wrapper               |
//! |-------------------------|----------------------------|
//! | `_sys_lwmutex_create`   | [`sys_lwmutex_create`]     |
//! | `_sys_lwmutex_destroy`  | [`sys_lwmutex_destroy`]    |
//! | `_sys_lwmutex_lock`     | [`sys_lwmutex_lock`]       |
//! | `_sys_lwmutex_unlock`   | [`sys_lwmutex_unlock`]     |
//! | `_sys_lwmutex_trylock`  | [`sys_lwmutex_trylock`]    |
//!
//! ## Blocking model
//!
//! Same contract as `rpcs3-lv2-sync`: [`LockOutcome::MustBlock`] tells
//! the emu core to park the caller; when the owner unlocks, the
//! kernel picks a waiter and the caller resumes with `Acquired`.

use rpcs3_emu_types::CellError;

// =====================================================================
// Sentinels + protocol constants
// =====================================================================

pub const LWMUTEX_FREE: u32 = 0xFFFF_FFFF;
pub const LWMUTEX_DEAD: u32 = 0xFFFF_FFFE;
pub const LWMUTEX_RESERVED: u32 = 0xFFFF_FFFD;

/// `SYS_SYNC_FIFO` — waiters woken FIFO (default).
pub const PROTOCOL_FIFO: u32 = 0x01;
/// `SYS_SYNC_PRIORITY` — waiters woken by priority.
pub const PROTOCOL_PRIORITY: u32 = 0x02;
/// `SYS_SYNC_PRIORITY_INHERIT` — accepted as alias of `PRIORITY`.
pub const PROTOCOL_PRIORITY_INHERIT: u32 = 0x03;
/// `SYS_SYNC_RETRY` — lwmutex-specific; user side retries on contention.
pub const PROTOCOL_RETRY: u32 = 0x04;

/// `sys_lwmutex` attribute flag for recursion.
pub const LWMUTEX_RECURSIVE: u32 = 0x02;

/// `SYS_SYNC_NOT_RECURSIVE` — PSL1GHT's not-recursive sentinel
/// (`<sys/lwmutex.h>`). Folded to `false` by [`LwMutexAttribute::parse`].
pub const ATTR_NOT_RECURSIVE_PSL1GHT: u32 = 0x10;
/// `SYS_SYNC_RECURSIVE` — PSL1GHT's recursive sentinel. Folded to
/// `true` by [`LwMutexAttribute::parse`].
pub const ATTR_RECURSIVE_PSL1GHT: u32 = 0x20;

/// PSL1GHT user-form protocol constants (`<sys/lwmutex.h>`).
/// Folded into the `0x01..=0x04` kernel form by [`LwMutexAttribute::parse`].
pub const ATTR_PROTOCOL_FIFO_PSL1GHT: u32 = 0x10;
pub const ATTR_PROTOCOL_PRIORITY_PSL1GHT: u32 = 0x20;
pub const ATTR_PROTOCOL_PRIORITY_INHERIT_PSL1GHT: u32 = 0x30;
pub const ATTR_PROTOCOL_RETRY_PSL1GHT: u32 = 0x40;

// =====================================================================
// User-memory attribute struct — byte-exact vs PSL1GHT `sys_lwmutex_attribute_t`
// =====================================================================

/// 16-byte BE control block the guest passes to `_sys_lwmutex_create`.
///
/// Layout from PSL1GHT `<sys/lwmutex.h>`:
/// ```text
/// 0x00  attr_protocol   u32 BE
/// 0x04  attr_recursive  u32 BE
/// 0x08  name            char[8]
/// ```
///
/// `attr_protocol` arrives in either of two encodings:
/// - **PSL1GHT user form**: `0x10`/`0x20`/`0x30`/`0x40` for
///   FIFO/PRIO/INHERIT/RETRY (the constants PSL1GHT exposes through
///   `SYS_SYNC_*_ATTR`).
/// - **Kernel form**: `0x01`/`0x02`/`0x03`/`0x04` (what
///   `validate_protocol` accepts directly).
///
/// `parse` folds both into the kernel form so downstream wrappers
/// always see a canonical value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LwMutexAttribute {
    /// Kernel-form protocol (0x01..=0x04). `parse` folds the PSL1GHT
    /// user form into this.
    pub protocol: u32,
    /// True when the attr requested a recursive mutex.
    pub recursive: bool,
    /// First 8 bytes of `name` as the guest provided them. Stored as a
    /// raw byte array because PSL1GHT doesn't require NUL termination
    /// (the field is fixed-length).
    pub name: [u8; 8],
}

impl LwMutexAttribute {
    /// Fixed wire size of the BE struct.
    pub const SIZE: usize = 16;

    /// Decode the 16-byte BE struct directly. Returns `EINVAL` if the
    /// folded protocol is unrecognised.
    pub fn parse(buf: &[u8; Self::SIZE]) -> Result<Self, CellError> {
        let raw_protocol = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let raw_recursive = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);

        let protocol = match raw_protocol {
            ATTR_PROTOCOL_FIFO_PSL1GHT | PROTOCOL_FIFO => PROTOCOL_FIFO,
            ATTR_PROTOCOL_PRIORITY_PSL1GHT | PROTOCOL_PRIORITY => PROTOCOL_PRIORITY,
            ATTR_PROTOCOL_PRIORITY_INHERIT_PSL1GHT | PROTOCOL_PRIORITY_INHERIT => {
                PROTOCOL_PRIORITY_INHERIT
            }
            ATTR_PROTOCOL_RETRY_PSL1GHT | PROTOCOL_RETRY => PROTOCOL_RETRY,
            _ => return Err(CellError::EINVAL),
        };

        // recursive: PSL1GHT user-form 0x20 OR kernel-form LWMUTEX_RECURSIVE,
        // anything else (including the 0x10 NOT_RECURSIVE) → false.
        let recursive = matches!(raw_recursive, ATTR_RECURSIVE_PSL1GHT | LWMUTEX_RECURSIVE);

        let mut name = [0u8; 8];
        name.copy_from_slice(&buf[8..16]);

        Ok(Self { protocol, recursive, name })
    }

    /// Default attr — FIFO, non-recursive, empty name. Useful for the
    /// `attr_ptr == 0` syscall case the dispatcher accepts as a
    /// shortcut.
    #[must_use]
    pub fn fifo_non_recursive() -> Self {
        Self { protocol: PROTOCOL_FIFO, recursive: false, name: [0u8; 8] }
    }
}

// =====================================================================
// User-memory control struct — byte-exact vs C++ `sys_lwmutex_t`
// =====================================================================

/// 32-byte BE control block kept in user memory. All fields are
/// big-endian; helpers below are endianness-aware.
///
/// Layout (offsets match C++):
/// ```text
/// 0x00  owner     u32 BE
/// 0x04  waiter    u32 BE
/// 0x08  attribute u32 BE  (protocol in low 8 bits, recursive flag bit 1)
/// 0x0C  rcount    u32 BE  (recursive count)
/// 0x10  sleep_q   u32 BE  (kernel lwmutex id)
/// 0x14  pad       u32 BE
/// 0x18  reserved  u64     (keeps struct at 32 bytes)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct LwMutexControl {
    pub owner_be: u32,
    pub waiter_be: u32,
    pub attribute_be: u32,
    pub rcount_be: u32,
    pub sleep_q_be: u32,
    pub pad0_be: u32,
    pub reserved: u64,
}

const _: () = {
    // Lock the struct size to 32 bytes so it stays ABI-compatible with
    // guest code that takes `sizeof(sys_lwmutex_t) == 32`.
    assert!(core::mem::size_of::<LwMutexControl>() == 32);
};

impl LwMutexControl {
    /// Build a fresh control block for a given protocol + recursive flag.
    #[must_use]
    pub fn new(protocol: u32, recursive: bool) -> Self {
        let attribute = protocol | if recursive { LWMUTEX_RECURSIVE } else { 0 };
        Self {
            owner_be: LWMUTEX_FREE.to_be(),
            waiter_be: 0u32.to_be(),
            attribute_be: attribute.to_be(),
            rcount_be: 0u32.to_be(),
            sleep_q_be: 0u32.to_be(),
            pad0_be: 0,
            reserved: 0,
        }
    }

    #[must_use]
    pub fn owner(&self) -> u32 {
        u32::from_be(self.owner_be)
    }
    pub fn set_owner(&mut self, v: u32) {
        self.owner_be = v.to_be();
    }

    #[must_use]
    pub fn waiter(&self) -> u32 {
        u32::from_be(self.waiter_be)
    }
    pub fn set_waiter(&mut self, v: u32) {
        self.waiter_be = v.to_be();
    }

    #[must_use]
    pub fn attribute(&self) -> u32 {
        u32::from_be(self.attribute_be)
    }

    #[must_use]
    pub fn protocol(&self) -> u32 {
        self.attribute() & 0xFF
    }

    #[must_use]
    pub fn is_recursive(&self) -> bool {
        self.attribute() & LWMUTEX_RECURSIVE != 0
    }

    #[must_use]
    pub fn rcount(&self) -> u32 {
        u32::from_be(self.rcount_be)
    }
    pub fn set_rcount(&mut self, v: u32) {
        self.rcount_be = v.to_be();
    }

    #[must_use]
    pub fn sleep_queue(&self) -> u32 {
        u32::from_be(self.sleep_q_be)
    }
    pub fn set_sleep_queue(&mut self, v: u32) {
        self.sleep_q_be = v.to_be();
    }

    /// Poison the word — guest code checks for this sentinel to decide
    /// it's safe to free the backing memory.
    pub fn mark_dead(&mut self) {
        self.owner_be = LWMUTEX_DEAD.to_be();
    }

    #[must_use]
    pub fn is_dead(&self) -> bool {
        self.owner() == LWMUTEX_DEAD
    }
}

// =====================================================================
// Outcomes
// =====================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockOutcome {
    /// Caller owns the mutex (fast path or recursive re-lock).
    Acquired,
    /// Caller must park on the kernel sleep queue.
    MustBlock,
    /// Non-blocking `trylock` failed because another thread holds it.
    Busy,
}

// =====================================================================
// Kernel-side trait — parks/resumes waiters, tracks sleep queue id
// =====================================================================

/// The kernel-side table of lwmutex sleep queues. The emu core
/// implements this; this crate only manipulates the user-memory
/// control struct + validates syscall semantics.
pub trait LwMutexTable {
    /// Allocate a new sleep queue for a control struct. Returns the
    /// `lwmutex_id` (`id_base = 0x95000000`).
    fn lwmutex_create(&mut self, protocol: u32) -> Result<u32, CellError>;

    /// Destroy a sleep queue. `EBUSY` if any thread is parked on it.
    fn lwmutex_destroy(&mut self, id: u32) -> Result<(), CellError>;

    /// Enqueue `tid` on the sleep queue of `id`.
    fn lwmutex_enqueue(&mut self, id: u32, tid: u32) -> Result<(), CellError>;

    /// Pop the next waiter (FIFO or priority per protocol). Returns
    /// `None` if the queue is empty (caller releases the mutex).
    fn lwmutex_dequeue(&mut self, id: u32) -> Result<Option<u32>, CellError>;

    /// Number of parked threads — used by `destroy` to return `EBUSY`.
    fn lwmutex_waiter_count(&self, id: u32) -> Result<u32, CellError>;
}

// =====================================================================
// Syscalls — user side operates on control struct, kernel side via trait
// =====================================================================

fn validate_protocol(protocol: u32) -> Result<(), CellError> {
    match protocol {
        PROTOCOL_FIFO | PROTOCOL_PRIORITY | PROTOCOL_PRIORITY_INHERIT | PROTOCOL_RETRY => Ok(()),
        _ => Err(CellError::EINVAL),
    }
}

/// `_sys_lwmutex_create(lwmutex_id_out, protocol, control_ptr, has_name, name)`.
///
/// Initialises the user-memory control struct in place and allocates
/// a kernel sleep queue.
#[must_use]
pub fn sys_lwmutex_create<T: LwMutexTable + ?Sized>(
    table: &mut T,
    protocol: u32,
    control: &mut LwMutexControl,
    recursive: bool,
) -> Result<u32, CellError> {
    validate_protocol(protocol)?;
    let id = table.lwmutex_create(protocol)?;
    *control = LwMutexControl::new(protocol, recursive);
    control.set_sleep_queue(id);
    Ok(id)
}

/// `_sys_lwmutex_destroy(lwmutex_id)`.
///
/// Poisons the user-memory word with `LWMUTEX_DEAD` *only if* the
/// kernel side reports no parked waiters. If any thread is parked,
/// returns `CELL_EBUSY` and leaves the struct untouched.
#[must_use]
pub fn sys_lwmutex_destroy<T: LwMutexTable + ?Sized>(
    table: &mut T,
    control: &mut LwMutexControl,
    id: u32,
) -> Result<(), CellError> {
    if table.lwmutex_waiter_count(id)? > 0 {
        return Err(CellError::EBUSY);
    }
    table.lwmutex_destroy(id)?;
    control.mark_dead();
    Ok(())
}

/// `_sys_lwmutex_lock(lwmutex_id, timeout_us)` on behalf of `tid`.
#[must_use]
pub fn sys_lwmutex_lock<T: LwMutexTable + ?Sized>(
    table: &mut T,
    control: &mut LwMutexControl,
    id: u32,
    tid: u32,
    _timeout_us: u64,
) -> Result<LockOutcome, CellError> {
    if control.is_dead() {
        return Err(CellError::ESRCH);
    }

    match control.owner() {
        LWMUTEX_FREE => {
            control.set_owner(tid);
            control.set_rcount(1);
            Ok(LockOutcome::Acquired)
        }
        current if current == tid => {
            // Recursive case — only valid if struct marked recursive.
            if !control.is_recursive() {
                return Err(CellError::EDEADLK);
            }
            let new_count = control
                .rcount()
                .checked_add(1)
                .ok_or(CellError::EKRESOURCE)?;
            control.set_rcount(new_count);
            Ok(LockOutcome::Acquired)
        }
        _ => {
            // Contended: publish waiter + park.
            control.set_waiter(control.waiter().saturating_add(1));
            table.lwmutex_enqueue(id, tid)?;
            Ok(LockOutcome::MustBlock)
        }
    }
}

/// `_sys_lwmutex_trylock(lwmutex_id)`.
#[must_use]
pub fn sys_lwmutex_trylock(
    control: &mut LwMutexControl,
    tid: u32,
) -> Result<LockOutcome, CellError> {
    if control.is_dead() {
        return Err(CellError::ESRCH);
    }
    match control.owner() {
        LWMUTEX_FREE => {
            control.set_owner(tid);
            control.set_rcount(1);
            Ok(LockOutcome::Acquired)
        }
        current if current == tid => {
            if !control.is_recursive() {
                return Err(CellError::EDEADLK);
            }
            let new_count = control
                .rcount()
                .checked_add(1)
                .ok_or(CellError::EKRESOURCE)?;
            control.set_rcount(new_count);
            Ok(LockOutcome::Acquired)
        }
        _ => Ok(LockOutcome::Busy),
    }
}

/// `_sys_lwmutex_unlock(lwmutex_id)` on behalf of `tid`. Returns the
/// tid of the waiter that was handed ownership (or `None` if no
/// waiters — the mutex is now free).
#[must_use]
pub fn sys_lwmutex_unlock<T: LwMutexTable + ?Sized>(
    table: &mut T,
    control: &mut LwMutexControl,
    id: u32,
    tid: u32,
) -> Result<Option<u32>, CellError> {
    if control.is_dead() {
        return Err(CellError::ESRCH);
    }
    if control.owner() != tid {
        return Err(CellError::EPERM);
    }

    // Recursive: decrement count; return still-held if >0.
    let count = control.rcount();
    if count == 0 {
        return Err(CellError::EPERM);
    }
    if count > 1 {
        control.set_rcount(count - 1);
        return Ok(None);
    }

    // Fully releasing: hand to next waiter or mark free.
    control.set_rcount(0);
    if let Some(next) = table.lwmutex_dequeue(id)? {
        control.set_owner(next);
        control.set_rcount(1);
        control.set_waiter(control.waiter().saturating_sub(1));
        Ok(Some(next))
    } else {
        control.set_owner(LWMUTEX_FREE);
        Ok(None)
    }
}

// =====================================================================
// Reference table implementation
// =====================================================================

#[derive(Debug, Default)]
pub struct TestLwMutexTable {
    next_id: u32,
    queues: std::collections::BTreeMap<u32, Queue>,
}

#[derive(Debug)]
struct Queue {
    protocol: u32,
    waiters: Vec<u32>,
}

impl TestLwMutexTable {
    #[must_use]
    pub fn queue_len(&self, id: u32) -> Option<usize> {
        self.queues.get(&id).map(|q| q.waiters.len())
    }
}

impl LwMutexTable for TestLwMutexTable {
    fn lwmutex_create(&mut self, protocol: u32) -> Result<u32, CellError> {
        self.next_id += 1;
        // Match C++ `lv2_lwmutex::id_base = 0x95000000`.
        let id = 0x9500_0000 | self.next_id;
        self.queues.insert(id, Queue { protocol, waiters: Vec::new() });
        Ok(id)
    }

    fn lwmutex_destroy(&mut self, id: u32) -> Result<(), CellError> {
        let q = self.queues.get(&id).ok_or(CellError::ESRCH)?;
        if !q.waiters.is_empty() {
            return Err(CellError::EBUSY);
        }
        self.queues.remove(&id);
        Ok(())
    }

    fn lwmutex_enqueue(&mut self, id: u32, tid: u32) -> Result<(), CellError> {
        let q = self.queues.get_mut(&id).ok_or(CellError::ESRCH)?;
        q.waiters.push(tid);
        Ok(())
    }

    fn lwmutex_dequeue(&mut self, id: u32) -> Result<Option<u32>, CellError> {
        let q = self.queues.get_mut(&id).ok_or(CellError::ESRCH)?;
        if q.waiters.is_empty() {
            return Ok(None);
        }
        // FIFO in this reference impl; priority-protocol would peek
        // at per-thread priority here.
        let _ = q.protocol;
        Ok(Some(q.waiters.remove(0)))
    }

    fn lwmutex_waiter_count(&self, id: u32) -> Result<u32, CellError> {
        let q = self.queues.get(&id).ok_or(CellError::ESRCH)?;
        Ok(q.waiters.len() as u32)
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- LwMutexAttribute parser (R10.1.c) ------------------------

    fn make_attr_bytes(protocol: u32, recursive: u32, name: &[u8; 8]) -> [u8; 16] {
        let mut buf = [0u8; 16];
        buf[0..4].copy_from_slice(&protocol.to_be_bytes());
        buf[4..8].copy_from_slice(&recursive.to_be_bytes());
        buf[8..16].copy_from_slice(name);
        buf
    }

    #[test]
    fn attr_parse_folds_psl1ght_fifo_not_recursive() {
        let bytes = make_attr_bytes(0x10, 0x10, b"lwlock\0\0");
        let attr = LwMutexAttribute::parse(&bytes).unwrap();
        assert_eq!(attr.protocol, PROTOCOL_FIFO);
        assert!(!attr.recursive);
        assert_eq!(&attr.name, b"lwlock\0\0");
    }

    #[test]
    fn attr_parse_folds_psl1ght_priority_recursive() {
        let bytes = make_attr_bytes(0x20, 0x20, &[0u8; 8]);
        let attr = LwMutexAttribute::parse(&bytes).unwrap();
        assert_eq!(attr.protocol, PROTOCOL_PRIORITY);
        assert!(attr.recursive);
    }

    #[test]
    fn attr_parse_accepts_kernel_form_directly() {
        let bytes = make_attr_bytes(0x03, LWMUTEX_RECURSIVE, &[b'k'; 8]);
        let attr = LwMutexAttribute::parse(&bytes).unwrap();
        assert_eq!(attr.protocol, PROTOCOL_PRIORITY_INHERIT);
        assert!(attr.recursive);
    }

    #[test]
    fn attr_parse_rejects_unknown_protocol() {
        let bytes = make_attr_bytes(0xDEAD, 0x10, &[0u8; 8]);
        assert_eq!(LwMutexAttribute::parse(&bytes), Err(CellError::EINVAL));
    }

    #[test]
    fn attr_default_is_fifo_non_recursive() {
        let a = LwMutexAttribute::fifo_non_recursive();
        assert_eq!(a.protocol, PROTOCOL_FIFO);
        assert!(!a.recursive);
        assert_eq!(a.name, [0u8; 8]);
    }

    #[test]
    fn attr_parse_retry_protocol() {
        let bytes = make_attr_bytes(0x40, 0x10, &[0u8; 8]);
        let attr = LwMutexAttribute::parse(&bytes).unwrap();
        assert_eq!(attr.protocol, PROTOCOL_RETRY);
    }

    // -- existing helpers -----------------------------------------

    fn setup() -> (TestLwMutexTable, LwMutexControl, u32) {
        let mut t = TestLwMutexTable::default();
        let mut ctrl = LwMutexControl::new(PROTOCOL_FIFO, false);
        let id = sys_lwmutex_create(&mut t, PROTOCOL_FIFO, &mut ctrl, false).unwrap();
        (t, ctrl, id)
    }

    #[test]
    fn control_struct_is_exactly_32_bytes() {
        assert_eq!(core::mem::size_of::<LwMutexControl>(), 32);
    }

    #[test]
    fn create_puts_id_in_0x95000000_range() {
        let (_t, _c, id) = setup();
        assert_eq!(id & 0xFF00_0000, 0x9500_0000);
    }

    #[test]
    fn create_initialises_control_to_free() {
        let (_t, ctrl, id) = setup();
        assert_eq!(ctrl.owner(), LWMUTEX_FREE);
        assert_eq!(ctrl.rcount(), 0);
        assert_eq!(ctrl.sleep_queue(), id);
    }

    #[test]
    fn create_rejects_bad_protocol() {
        let mut t = TestLwMutexTable::default();
        let mut ctrl = LwMutexControl::new(PROTOCOL_FIFO, false);
        let err = sys_lwmutex_create(&mut t, 0x99, &mut ctrl, false).unwrap_err();
        assert_eq!(err, CellError::EINVAL);
    }

    #[test]
    fn create_accepts_retry_protocol() {
        let mut t = TestLwMutexTable::default();
        let mut ctrl = LwMutexControl::new(PROTOCOL_FIFO, false);
        sys_lwmutex_create(&mut t, PROTOCOL_RETRY, &mut ctrl, false).unwrap();
        assert_eq!(ctrl.protocol(), PROTOCOL_RETRY);
    }

    #[test]
    fn fast_path_lock_sets_owner() {
        let (mut t, mut ctrl, id) = setup();
        let out = sys_lwmutex_lock(&mut t, &mut ctrl, id, 42, 0).unwrap();
        assert_eq!(out, LockOutcome::Acquired);
        assert_eq!(ctrl.owner(), 42);
        assert_eq!(ctrl.rcount(), 1);
    }

    #[test]
    fn trylock_busy_when_held() {
        let (mut t, mut ctrl, id) = setup();
        sys_lwmutex_lock(&mut t, &mut ctrl, id, 42, 0).unwrap();
        let out = sys_lwmutex_trylock(&mut ctrl, 7).unwrap();
        assert_eq!(out, LockOutcome::Busy);
        assert_eq!(ctrl.owner(), 42);
    }

    #[test]
    fn lock_contended_enqueues_waiter_and_returns_must_block() {
        let (mut t, mut ctrl, id) = setup();
        sys_lwmutex_lock(&mut t, &mut ctrl, id, 42, 0).unwrap();
        let out = sys_lwmutex_lock(&mut t, &mut ctrl, id, 7, 0).unwrap();
        assert_eq!(out, LockOutcome::MustBlock);
        assert_eq!(ctrl.waiter(), 1);
        assert_eq!(t.queue_len(id), Some(1));
    }

    #[test]
    fn non_recursive_relock_by_same_tid_is_edeadlk() {
        let (mut t, mut ctrl, id) = setup();
        sys_lwmutex_lock(&mut t, &mut ctrl, id, 42, 0).unwrap();
        let err = sys_lwmutex_lock(&mut t, &mut ctrl, id, 42, 0).unwrap_err();
        assert_eq!(err, CellError::EDEADLK);
    }

    #[test]
    fn recursive_relock_by_same_tid_increments_count() {
        let mut t = TestLwMutexTable::default();
        let mut ctrl = LwMutexControl::new(PROTOCOL_FIFO, true);
        let id = sys_lwmutex_create(&mut t, PROTOCOL_FIFO, &mut ctrl, true).unwrap();

        sys_lwmutex_lock(&mut t, &mut ctrl, id, 42, 0).unwrap();
        sys_lwmutex_lock(&mut t, &mut ctrl, id, 42, 0).unwrap();
        sys_lwmutex_lock(&mut t, &mut ctrl, id, 42, 0).unwrap();
        assert_eq!(ctrl.rcount(), 3);

        // Unlocks unwind.
        assert_eq!(sys_lwmutex_unlock(&mut t, &mut ctrl, id, 42).unwrap(), None);
        assert_eq!(ctrl.rcount(), 2);
        sys_lwmutex_unlock(&mut t, &mut ctrl, id, 42).unwrap();
        sys_lwmutex_unlock(&mut t, &mut ctrl, id, 42).unwrap();
        assert_eq!(ctrl.owner(), LWMUTEX_FREE);
        assert_eq!(ctrl.rcount(), 0);
    }

    #[test]
    fn unlock_by_wrong_tid_is_eperm() {
        let (mut t, mut ctrl, id) = setup();
        sys_lwmutex_lock(&mut t, &mut ctrl, id, 42, 0).unwrap();
        let err = sys_lwmutex_unlock(&mut t, &mut ctrl, id, 7).unwrap_err();
        assert_eq!(err, CellError::EPERM);
    }

    #[test]
    fn unlock_hands_ownership_to_next_waiter() {
        let (mut t, mut ctrl, id) = setup();
        sys_lwmutex_lock(&mut t, &mut ctrl, id, 42, 0).unwrap();
        sys_lwmutex_lock(&mut t, &mut ctrl, id, 7, 0).unwrap(); // MustBlock
        sys_lwmutex_lock(&mut t, &mut ctrl, id, 8, 0).unwrap(); // MustBlock

        let next = sys_lwmutex_unlock(&mut t, &mut ctrl, id, 42).unwrap();
        assert_eq!(next, Some(7));
        assert_eq!(ctrl.owner(), 7);
        assert_eq!(ctrl.waiter(), 1);

        let next = sys_lwmutex_unlock(&mut t, &mut ctrl, id, 7).unwrap();
        assert_eq!(next, Some(8));
        assert_eq!(ctrl.owner(), 8);
        assert_eq!(ctrl.waiter(), 0);

        let next = sys_lwmutex_unlock(&mut t, &mut ctrl, id, 8).unwrap();
        assert_eq!(next, None);
        assert_eq!(ctrl.owner(), LWMUTEX_FREE);
    }

    #[test]
    fn destroy_while_held_free_mutex_poisons_control() {
        let (mut t, mut ctrl, id) = setup();
        sys_lwmutex_destroy(&mut t, &mut ctrl, id).unwrap();
        assert_eq!(ctrl.owner(), LWMUTEX_DEAD);
        assert!(ctrl.is_dead());
    }

    #[test]
    fn destroy_with_parked_waiters_is_ebusy() {
        let (mut t, mut ctrl, id) = setup();
        sys_lwmutex_lock(&mut t, &mut ctrl, id, 42, 0).unwrap();
        sys_lwmutex_lock(&mut t, &mut ctrl, id, 7, 0).unwrap();
        let err = sys_lwmutex_destroy(&mut t, &mut ctrl, id).unwrap_err();
        assert_eq!(err, CellError::EBUSY);
        assert!(!ctrl.is_dead(), "control must be untouched when destroy fails");
    }

    #[test]
    fn lock_after_dead_is_esrch() {
        let (mut t, mut ctrl, id) = setup();
        sys_lwmutex_destroy(&mut t, &mut ctrl, id).unwrap();
        let err = sys_lwmutex_lock(&mut t, &mut ctrl, id, 42, 0).unwrap_err();
        assert_eq!(err, CellError::ESRCH);
    }

    #[test]
    fn trylock_after_dead_is_esrch() {
        let (mut t, mut ctrl, id) = setup();
        sys_lwmutex_destroy(&mut t, &mut ctrl, id).unwrap();
        let err = sys_lwmutex_trylock(&mut ctrl, 42).unwrap_err();
        assert_eq!(err, CellError::ESRCH);
    }

    #[test]
    fn unlock_after_dead_is_esrch() {
        let (mut t, mut ctrl, id) = setup();
        sys_lwmutex_destroy(&mut t, &mut ctrl, id).unwrap();
        let err = sys_lwmutex_unlock(&mut t, &mut ctrl, id, 42).unwrap_err();
        assert_eq!(err, CellError::ESRCH);
    }

    #[test]
    fn control_is_big_endian_byte_layout() {
        // Prove the struct is byte-exact vs guest memory: write owner
        // via the wrapper and inspect the raw bytes.
        let mut c = LwMutexControl::new(PROTOCOL_FIFO, false);
        c.set_owner(0xAABB_CCDD);
        let raw = unsafe {
            core::slice::from_raw_parts(
                (&c as *const LwMutexControl).cast::<u8>(),
                core::mem::size_of::<LwMutexControl>(),
            )
        };
        assert_eq!(&raw[0..4], &[0xAA, 0xBB, 0xCC, 0xDD]);
    }

    #[test]
    fn unlock_releases_when_rcount_1_no_waiters() {
        let (mut t, mut ctrl, id) = setup();
        sys_lwmutex_lock(&mut t, &mut ctrl, id, 42, 0).unwrap();
        let out = sys_lwmutex_unlock(&mut t, &mut ctrl, id, 42).unwrap();
        assert_eq!(out, None);
        assert_eq!(ctrl.owner(), LWMUTEX_FREE);
        assert_eq!(ctrl.rcount(), 0);
    }

    #[test]
    fn recursive_unlock_mid_stack_still_blocks_others() {
        let mut t = TestLwMutexTable::default();
        let mut ctrl = LwMutexControl::new(PROTOCOL_FIFO, true);
        let id = sys_lwmutex_create(&mut t, PROTOCOL_FIFO, &mut ctrl, true).unwrap();

        sys_lwmutex_lock(&mut t, &mut ctrl, id, 42, 0).unwrap();
        sys_lwmutex_lock(&mut t, &mut ctrl, id, 42, 0).unwrap();
        assert_eq!(ctrl.rcount(), 2);

        // Other thread must still block even though only one release
        // has happened (because we're still recursive-locked).
        let out = sys_lwmutex_lock(&mut t, &mut ctrl, id, 7, 0).unwrap();
        assert_eq!(out, LockOutcome::MustBlock);

        sys_lwmutex_unlock(&mut t, &mut ctrl, id, 42).unwrap();
        // Still owned by 42 (rcount was 2).
        assert_eq!(ctrl.owner(), 42);
        assert_eq!(ctrl.rcount(), 1);

        // Now final release hands to waiter.
        let next = sys_lwmutex_unlock(&mut t, &mut ctrl, id, 42).unwrap();
        assert_eq!(next, Some(7));
    }

    #[test]
    fn trylock_recursive_same_tid_increments() {
        let mut t = TestLwMutexTable::default();
        let mut ctrl = LwMutexControl::new(PROTOCOL_FIFO, true);
        let id = sys_lwmutex_create(&mut t, PROTOCOL_FIFO, &mut ctrl, true).unwrap();
        sys_lwmutex_lock(&mut t, &mut ctrl, id, 42, 0).unwrap();
        let out = sys_lwmutex_trylock(&mut ctrl, 42).unwrap();
        assert_eq!(out, LockOutcome::Acquired);
        assert_eq!(ctrl.rcount(), 2);
    }

    #[test]
    fn attribute_byte_layout_encodes_protocol_and_recursive() {
        let c = LwMutexControl::new(PROTOCOL_PRIORITY, true);
        assert_eq!(c.protocol(), PROTOCOL_PRIORITY);
        assert!(c.is_recursive());
        assert_eq!(c.attribute(), PROTOCOL_PRIORITY | LWMUTEX_RECURSIVE);
    }
}
