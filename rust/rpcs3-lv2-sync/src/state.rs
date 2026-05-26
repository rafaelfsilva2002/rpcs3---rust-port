//! Concrete LV2 sync state container (R10.1.a).
//!
//! Where [`crate::SyncTable`] and the per-primitive crates
//! ([`rpcs3-lv2-lwmutex`](../../rpcs3-lv2-lwmutex/index.html), etc.)
//! define *protocol* (what the syscalls do via trait dispatch), this
//! module defines the *runtime* — a single owned registry that the
//! emulator core embeds per `EmuCore::run`. State is per-instance,
//! never global.
//!
//! ## Identity model
//!
//! Handles are wrapped in a [`Lv2SyncId`] newtype carrying a strongly
//! typed [`Lv2SyncKind`]. A raw u32 from guest code is paired with the
//! caller's expectation; the lookup helpers ([`Lv2SyncState::get`] /
//! [`Lv2SyncState::get_mut`]) enforce that the stored kind matches the
//! requested kind, returning [`CellError::EINVAL`] on mismatch so a
//! guest can't (e.g.) treat a semaphore handle as a mutex handle.
//!
//! ## Allocator
//!
//! Deterministic and monotonic: every [`Lv2SyncState::allocate`] returns
//! the next u32 starting at `1`. `0` is reserved as a "null handle"
//! sentinel. No ID reuse — destroyed entries leave a gap, but their ID
//! never comes back. This matches RPCS3 C++'s behavior for the sync
//! ID space and keeps capture traces deterministic.
//!
//! ## Scope of this slice (R10.1.a)
//!
//! - `LwMutex` (kernel-side waiter queue + owner record) is the only
//!   primitive container implemented here.
//! - Full lock/unlock semantics live in `rpcs3-lv2-lwmutex` (which
//!   continues to own the user-memory [`LwMutexControl`] BE struct);
//!   R10.1.a only delivers the kernel-side entry the next slices wire
//!   into `LwMutexTable`.
//! - Other primitives (mutex, sema, cond, event, rwlock) get their
//!   `Lv2SyncKind` variant pre-allocated so later slices add them
//!   without touching the registry shape.

use std::collections::{BTreeMap, VecDeque};

use rpcs3_emu_types::CellError;
use rpcs3_lv2_cond::{CondAttr, CondRegistry, WaitOutcome};
use rpcs3_lv2_event_flag::{
    EventFlagAttr, EventFlagRegistry, WaitOutcome as EvfWaitOutcome,
    WAIT_AND, WAIT_CLEAR, WAIT_CLEAR_ALL, WAIT_OR, WAITER_SINGLE,
};
use rpcs3_lv2_lwmutex::LwMutexTable;
use rpcs3_lv2_rwlock::{
    LockOutcome as RwLockOutcome, RwlockAttr, RwlockRegistry,
};
use rpcs3_lv2_event::{
    Event, EventRegistry, QueueAttr, ReceiveOutcome, QUEUE_DESTROY_FORCE,
};

use crate::{BlockOutcome, MutexAttr, SemaAttr, SyncTable};

/// High-byte tag for kernel lwmutex IDs. Matches RPCS3 C++'s
/// `lv2_lwmutex::id_base = 0x95000000`. Used by the [`LwMutexTable`]
/// impl on [`Lv2SyncState`] to tag the registry's internal counter
/// before exposing the handle to guest code, so capture traces match
/// what RPCS3 C++ would have emitted.
pub const LWMUTEX_ID_BASE: u32 = 0x9500_0000;
const LWMUTEX_ID_MASK: u32 = 0x00FF_FFFF;

// ====================================================================
// Id newtype + kind enum
// ====================================================================

/// Strongly-typed LV2 sync handle. Wraps the raw u32 guest sees and
/// carries the [`Lv2SyncKind`] the registry stored it under.
///
/// The kind is included so callers can pattern-match on a handle's
/// type without a round-trip through the registry. The raw u32 is what
/// gets written back to guest memory by the syscall wrappers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Lv2SyncId {
    raw: u32,
    kind: Lv2SyncKind,
}

impl Lv2SyncId {
    /// Raw u32 handle as the guest sees it.
    #[inline]
    #[must_use]
    pub fn raw(self) -> u32 {
        self.raw
    }

    /// The kind this handle was allocated under.
    #[inline]
    #[must_use]
    pub fn kind(self) -> Lv2SyncKind {
        self.kind
    }
}

/// Discriminator for what's stored under a given handle.
///
/// Pre-allocates one variant per LV2 sync primitive family planned for
/// R10. Only [`Lv2SyncKind::LwMutex`] is wired through to a container
/// in R10.1.a; the others reserve identity so later slices add them
/// without renumbering or breaking enum-exhaustiveness checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lv2SyncKind {
    /// `sys_lwmutex_t` — userspace-cooperative lock (R10.1).
    LwMutex,
    /// `sys_mutex_t` — kernel-side recursive lock (R10.2, reserved).
    Mutex,
    /// `sys_semaphore_t` — counting semaphore (R10.4, reserved).
    Sema,
    /// `sys_cond_t` — condition variable (R10.3, reserved).
    Cond,
    /// `sys_lwcond_t` — lightweight cond var (R10.8, reserved).
    LwCond,
    /// `sys_event_flag_t` — bitmask event flag set (R10.5, reserved).
    EventFlag,
    /// `sys_event_queue_t` — kernel event queue (R10.6, reserved).
    EventQueue,
    /// `sys_event_port_t` — event sender side (R10.6, reserved).
    EventPort,
    /// `sys_rwlock_t` — readers/writer lock (R10.7, reserved).
    RwLock,
}

// ====================================================================
// LwMutex kernel-side entry
// ====================================================================

/// `sys_lwmutex_attr_t` subset that matters for the kernel-side entry.
///
/// Mirrors the attr fields the kernel keeps post-create. The full BE
/// 12-byte struct lives in `rpcs3-lv2-lwmutex::LwMutexAttribute` (R10.1.b
/// wires the parse path).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LwMutexAttr {
    /// `SYS_SYNC_FIFO` / `SYS_SYNC_PRIORITY` / `SYS_SYNC_PRIORITY_INHERIT`
    /// / `SYS_SYNC_RETRY` — accepted as a raw u32 here; validation
    /// lives in the crate-level syscall wrapper.
    pub protocol: u32,
    /// True when `SYS_SYNC_RECURSIVE` is set in the attr's recursive
    /// field.
    pub recursive: bool,
}

/// Kernel-side state for one `sys_lwmutex_t` handle.
///
/// The userspace control word ([`crate::LwMutexControl`] in
/// `rpcs3-lv2-lwmutex`) is the source of truth for "is anyone holding
/// this right now"; this container is the kernel-side waiter queue and
/// recursion bookkeeping. R10.1.a delivers the container shape only —
/// `_sys_lwmutex_lock` / `_unlock` wiring is R10.1.b.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LwMutex {
    /// Attributes captured at create time.
    pub attr: LwMutexAttr,
    /// Owning thread id, or `None` when free.
    pub owner: Option<u32>,
    /// Recursive lock depth — 0 when free, ≥1 when held. For
    /// non-recursive lwmutexes this stays at 1 while held.
    pub recursion_count: u32,
    /// FIFO queue of parked thread ids waiting on the lwmutex.
    /// VecDeque so push_back + pop_front are O(1).
    pub waiters: VecDeque<u32>,
}

// ====================================================================
// Kernel sys_mutex entry (R10.2)
// ====================================================================

/// Kernel-side state for one `sys_mutex_t` handle.
///
/// Unlike [`LwMutex`], the entire state lives in the kernel — there is
/// no userspace control word. Ownership, recursion depth, and waiter
/// queue all live here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mutex {
    pub attr: MutexAttr,
    pub owner: Option<u32>,
    pub recursion_count: u32,
    pub waiters: VecDeque<u32>,
}

// ====================================================================
// Kernel sys_semaphore entry (R10.4)
// ====================================================================

/// Kernel-side state for one `sys_semaphore_t` handle.
///
/// Counting semaphore. `value` decrements on wait, increments on post.
/// `max` is the cap enforced by `sys_semaphore_post` (overflow returns
/// `EAGAIN`). Waiters parked when `value == 0` go in FIFO order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sema {
    pub attr: SemaAttr,
    pub value: i32,
    pub max: i32,
    pub waiters: VecDeque<u32>,
}

// ====================================================================
// Kernel sys_cond entry (R10.3)
// ====================================================================

// ====================================================================
// Kernel sys_event_flag entry (R10.5)
// ====================================================================

/// One parked waiter on a `sys_event_flag_t`. Carries the bit pattern
/// and mode it was waiting on so a subsequent `evflag_set` can match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventFlagWaiter {
    pub tid: u32,
    pub bitptn: u64,
    pub mode: u32,
}

/// Kernel-side state for one `sys_event_flag_t`.
#[derive(Debug, Clone, Default)]
pub struct EventFlag {
    pub attr: EventFlagAttr,
    pub pattern: u64,
    pub waiters: Vec<EventFlagWaiter>,
}

// ====================================================================
// Kernel sys_event_queue + sys_event_port entries (R10.6)
// ====================================================================

/// Kernel-side state for one `sys_event_queue_t`. Bounded FIFO of
/// `Event` tuples.
#[derive(Debug, Clone, Default)]
pub struct EventQueue {
    pub attr: QueueAttr,
    /// Max events the queue will hold. `port_send` to a full queue
    /// returns `EBUSY` (matching C++ `sys_event_port_send`).
    pub size: u32,
    pub pending: VecDeque<Event>,
}

/// Kernel-side state for one `sys_event_port_t`. May be connected to a
/// single event queue.
#[derive(Debug, Clone, Default)]
pub struct EventPort {
    pub port_type: u32,
    pub name: u64,
    /// Untagged registry id of the queue this port is bound to (None
    /// when disconnected).
    pub connected_queue_untagged: Option<u32>,
}

// ====================================================================
// Kernel sys_rwlock entry (R10.7)
// ====================================================================

/// Kernel-side state for one `sys_rwlock_t`. Tracks reader list +
/// optional writer + separate FIFO queues for read/write waiters.
/// PS3 LV2 is writer-priority — readers must block when a writer is
/// queued.
#[derive(Debug, Clone, Default)]
pub struct Rwlock {
    pub attr: RwlockAttr,
    pub readers: Vec<u32>,
    pub writer: Option<u32>,
    pub read_waiters: VecDeque<u32>,
    pub write_waiters: VecDeque<u32>,
}

/// Kernel-side state for one `sys_cond_t` (condition variable) handle.
///
/// Conds are always tied to a mutex (`mutex_id_untagged` is the raw
/// Lv2SyncState counter of the bound [`Mutex`]). `waiters` are threads
/// currently parked in `cond_wait`; `awakened` is threads a signal has
/// moved off the wait queue but which haven't yet successfully
/// reacquired the mutex.
#[derive(Debug, Clone, Default)]
pub struct Cond {
    pub attr: CondAttr,
    /// Untagged registry id of the bound mutex.
    pub mutex_id_untagged: u32,
    pub waiters: VecDeque<u32>,
    pub awakened: VecDeque<u32>,
}

// ====================================================================
// Registry entry — one per allocated handle
// ====================================================================

/// Storage variant for the [`Lv2SyncState`] map.
///
/// One variant per [`Lv2SyncKind`]. R10.1.a populates `LwMutex` only;
/// the others are pre-declared so future slices add a single arm to
/// each match without churning the enum.
#[derive(Debug)]
enum Entry {
    LwMutex(LwMutex),
    Mutex(Mutex),
    Sema(Sema),
    Cond(Cond),
    EventFlag(EventFlag),
    RwLock(Rwlock),
    EventQueue(EventQueue),
    EventPort(EventPort),
    // Variants below are placeholders the future slices flesh out.
    LwCond,
}

impl Entry {
    fn kind(&self) -> Lv2SyncKind {
        match self {
            Entry::LwMutex(_) => Lv2SyncKind::LwMutex,
            Entry::Mutex(_) => Lv2SyncKind::Mutex,
            Entry::Sema(_) => Lv2SyncKind::Sema,
            Entry::Cond(_) => Lv2SyncKind::Cond,
            Entry::EventFlag(_) => Lv2SyncKind::EventFlag,
            Entry::RwLock(_) => Lv2SyncKind::RwLock,
            Entry::EventQueue(_) => Lv2SyncKind::EventQueue,
            Entry::EventPort(_) => Lv2SyncKind::EventPort,
            Entry::LwCond => Lv2SyncKind::LwCond,
        }
    }

    fn new(kind: Lv2SyncKind) -> Self {
        match kind {
            Lv2SyncKind::LwMutex => Entry::LwMutex(LwMutex::default()),
            Lv2SyncKind::Mutex => Entry::Mutex(Mutex::default()),
            Lv2SyncKind::Sema => Entry::Sema(Sema::default()),
            Lv2SyncKind::Cond => Entry::Cond(Cond::default()),
            Lv2SyncKind::EventFlag => Entry::EventFlag(EventFlag::default()),
            Lv2SyncKind::RwLock => Entry::RwLock(Rwlock::default()),
            Lv2SyncKind::EventQueue => Entry::EventQueue(EventQueue::default()),
            Lv2SyncKind::EventPort => Entry::EventPort(EventPort::default()),
            Lv2SyncKind::LwCond => Entry::LwCond,
        }
    }
}

// ====================================================================
// Registry itself
// ====================================================================

/// Per-`EmuCore::run` registry of LV2 sync primitives.
///
/// One instance per emulator run. State is owned; no `static`, no
/// `OnceCell`, no thread-locals. The emu core embeds this and threads
/// it through syscall arms.
///
/// Lookups validate kind: passing a [`Lv2SyncKind::Sema`] handle to a
/// `LwMutex` getter returns `Err(CellError::EINVAL)`. Unknown ids
/// return `Err(CellError::ESRCH)`.
#[derive(Debug, Default)]
pub struct Lv2SyncState {
    /// `id_counter` is the most recently allocated raw u32. `0` is
    /// reserved as null/uninitialized; the first allocation produces
    /// `1`.
    id_counter: u32,
    /// BTreeMap keeps iteration order deterministic for tests + traces;
    /// HashMap would suffice for production lookup but ordering matters
    /// when behavior-freeze captures iterate handles for state diff.
    entries: BTreeMap<u32, Entry>,
}

impl Lv2SyncState {
    /// Fresh empty registry — typically called once per
    /// `EmuCore::new`.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a new handle of the requested kind. Always succeeds in
    /// R10.1.a (the only failure mode would be u32 overflow after
    /// `u32::MAX - 1` allocations — out of scope for any plausible
    /// emulator run).
    #[must_use]
    pub fn allocate(&mut self, kind: Lv2SyncKind) -> Lv2SyncId {
        // Monotonic. We never reuse ids — the C++ side does, but for
        // capture determinism + tracing clarity we keep it simple.
        self.id_counter = self
            .id_counter
            .checked_add(1)
            .expect("Lv2SyncState id counter overflowed u32");
        let raw = self.id_counter;
        self.entries.insert(raw, Entry::new(kind));
        Lv2SyncId { raw, kind }
    }

    /// Number of currently-allocated handles. Useful for tests + leak
    /// detection between runs.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no handles are alive.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Destroy a handle. Returns `Err(ESRCH)` for an unknown id and
    /// `Err(EINVAL)` for a kind mismatch.
    ///
    /// Future slices may layer "destroy with waiters → EBUSY" semantics
    /// (matching RPCS3 C++). R10.1.a keeps destroy unconditional so the
    /// registry shape is exercised without the per-primitive
    /// destruction guard, which lives at the lwmutex/mutex layer.
    pub fn destroy(
        &mut self,
        raw: u32,
        kind: Lv2SyncKind,
    ) -> Result<(), CellError> {
        let entry = self.entries.get(&raw).ok_or(CellError::ESRCH)?;
        if entry.kind() != kind {
            return Err(CellError::EINVAL);
        }
        self.entries.remove(&raw);
        Ok(())
    }

    // -- LwMutex accessors -----------------------------------------

    /// Borrow a [`LwMutex`] entry. Mismatched kind → `EINVAL`, unknown
    /// id → `ESRCH`.
    pub fn lw_mutex(&self, raw: u32) -> Result<&LwMutex, CellError> {
        match self.entries.get(&raw) {
            None => Err(CellError::ESRCH),
            Some(Entry::LwMutex(m)) => Ok(m),
            Some(_) => Err(CellError::EINVAL),
        }
    }

    /// Mutably borrow a [`LwMutex`] entry. Same error semantics as
    /// [`Self::lw_mutex`].
    pub fn lw_mutex_mut(
        &mut self,
        raw: u32,
    ) -> Result<&mut LwMutex, CellError> {
        match self.entries.get_mut(&raw) {
            None => Err(CellError::ESRCH),
            Some(Entry::LwMutex(m)) => Ok(m),
            Some(_) => Err(CellError::EINVAL),
        }
    }

    // -- Mutex accessors (R10.2) -----------------------------------

    pub fn mutex(&self, raw: u32) -> Result<&Mutex, CellError> {
        match self.entries.get(&raw) {
            None => Err(CellError::ESRCH),
            Some(Entry::Mutex(m)) => Ok(m),
            Some(_) => Err(CellError::EINVAL),
        }
    }

    pub fn mutex_mut(&mut self, raw: u32) -> Result<&mut Mutex, CellError> {
        match self.entries.get_mut(&raw) {
            None => Err(CellError::ESRCH),
            Some(Entry::Mutex(m)) => Ok(m),
            Some(_) => Err(CellError::EINVAL),
        }
    }

    // -- Sema accessors (R10.4) ------------------------------------

    pub fn sema(&self, raw: u32) -> Result<&Sema, CellError> {
        match self.entries.get(&raw) {
            None => Err(CellError::ESRCH),
            Some(Entry::Sema(s)) => Ok(s),
            Some(_) => Err(CellError::EINVAL),
        }
    }

    pub fn sema_mut(&mut self, raw: u32) -> Result<&mut Sema, CellError> {
        match self.entries.get_mut(&raw) {
            None => Err(CellError::ESRCH),
            Some(Entry::Sema(s)) => Ok(s),
            Some(_) => Err(CellError::EINVAL),
        }
    }

    // -- Cond accessors (R10.3) ------------------------------------

    pub fn cond(&self, raw: u32) -> Result<&Cond, CellError> {
        match self.entries.get(&raw) {
            None => Err(CellError::ESRCH),
            Some(Entry::Cond(c)) => Ok(c),
            Some(_) => Err(CellError::EINVAL),
        }
    }

    pub fn cond_mut(&mut self, raw: u32) -> Result<&mut Cond, CellError> {
        match self.entries.get_mut(&raw) {
            None => Err(CellError::ESRCH),
            Some(Entry::Cond(c)) => Ok(c),
            Some(_) => Err(CellError::EINVAL),
        }
    }

    // -- EventFlag accessors (R10.5) -------------------------------

    pub fn event_flag(&self, raw: u32) -> Result<&EventFlag, CellError> {
        match self.entries.get(&raw) {
            None => Err(CellError::ESRCH),
            Some(Entry::EventFlag(f)) => Ok(f),
            Some(_) => Err(CellError::EINVAL),
        }
    }

    pub fn event_flag_mut(
        &mut self,
        raw: u32,
    ) -> Result<&mut EventFlag, CellError> {
        match self.entries.get_mut(&raw) {
            None => Err(CellError::ESRCH),
            Some(Entry::EventFlag(f)) => Ok(f),
            Some(_) => Err(CellError::EINVAL),
        }
    }

    // -- Rwlock accessors (R10.7) ----------------------------------

    pub fn rwlock(&self, raw: u32) -> Result<&Rwlock, CellError> {
        match self.entries.get(&raw) {
            None => Err(CellError::ESRCH),
            Some(Entry::RwLock(r)) => Ok(r),
            Some(_) => Err(CellError::EINVAL),
        }
    }

    pub fn rwlock_mut(&mut self, raw: u32) -> Result<&mut Rwlock, CellError> {
        match self.entries.get_mut(&raw) {
            None => Err(CellError::ESRCH),
            Some(Entry::RwLock(r)) => Ok(r),
            Some(_) => Err(CellError::EINVAL),
        }
    }

    // -- EventQueue accessors (R10.6) ------------------------------

    pub fn event_queue(&self, raw: u32) -> Result<&EventQueue, CellError> {
        match self.entries.get(&raw) {
            None => Err(CellError::ESRCH),
            Some(Entry::EventQueue(q)) => Ok(q),
            Some(_) => Err(CellError::EINVAL),
        }
    }

    pub fn event_queue_mut(
        &mut self,
        raw: u32,
    ) -> Result<&mut EventQueue, CellError> {
        match self.entries.get_mut(&raw) {
            None => Err(CellError::ESRCH),
            Some(Entry::EventQueue(q)) => Ok(q),
            Some(_) => Err(CellError::EINVAL),
        }
    }

    // -- EventPort accessors (R10.6) -------------------------------

    pub fn event_port(&self, raw: u32) -> Result<&EventPort, CellError> {
        match self.entries.get(&raw) {
            None => Err(CellError::ESRCH),
            Some(Entry::EventPort(p)) => Ok(p),
            Some(_) => Err(CellError::EINVAL),
        }
    }

    pub fn event_port_mut(
        &mut self,
        raw: u32,
    ) -> Result<&mut EventPort, CellError> {
        match self.entries.get_mut(&raw) {
            None => Err(CellError::ESRCH),
            Some(Entry::EventPort(p)) => Ok(p),
            Some(_) => Err(CellError::EINVAL),
        }
    }
}

// ====================================================================
// LwMutexTable bridge (R10.1.b)
// ====================================================================

impl Lv2SyncState {
    /// Convert a raw counter into the kernel-visible lwmutex id by
    /// applying the `0x95000000` tag. Used at the [`LwMutexTable`]
    /// boundary so guest code sees the same id space RPCS3 C++ would
    /// have emitted.
    #[inline]
    fn lwmutex_tag(raw: u32) -> u32 {
        LWMUTEX_ID_BASE | (raw & LWMUTEX_ID_MASK)
    }

    /// Reverse of [`Self::lwmutex_tag`] — strip the kind tag back to
    /// the registry counter.
    #[inline]
    fn lwmutex_untag(id: u32) -> u32 {
        id & LWMUTEX_ID_MASK
    }
}

impl LwMutexTable for Lv2SyncState {
    fn lwmutex_create(&mut self, protocol: u32) -> Result<u32, CellError> {
        let id = self.allocate(Lv2SyncKind::LwMutex);
        // Set the create-time attr so dequeue can later honor protocol.
        // Recursive flag is not part of the kernel side — it lives in
        // the userspace control word — so leave it `false` here.
        self.lw_mutex_mut(id.raw())?.attr = LwMutexAttr {
            protocol,
            recursive: false,
        };
        Ok(Self::lwmutex_tag(id.raw()))
    }

    fn lwmutex_destroy(&mut self, id: u32) -> Result<(), CellError> {
        let raw = Self::lwmutex_untag(id);
        // Mirror the C++ guard: refuse destroy while any waiter is
        // parked. The trait contract says EBUSY, not ESRCH.
        if self.lw_mutex(raw)?.waiters.is_empty() {
            self.destroy(raw, Lv2SyncKind::LwMutex)
        } else {
            Err(CellError::EBUSY)
        }
    }

    fn lwmutex_enqueue(&mut self, id: u32, tid: u32) -> Result<(), CellError> {
        let raw = Self::lwmutex_untag(id);
        self.lw_mutex_mut(raw)?.waiters.push_back(tid);
        Ok(())
    }

    fn lwmutex_dequeue(&mut self, id: u32) -> Result<Option<u32>, CellError> {
        let raw = Self::lwmutex_untag(id);
        Ok(self.lw_mutex_mut(raw)?.waiters.pop_front())
    }

    fn lwmutex_waiter_count(&self, id: u32) -> Result<u32, CellError> {
        let raw = Self::lwmutex_untag(id);
        let n = self.lw_mutex(raw)?.waiters.len();
        // u32 cast is safe: lwmutex queues are PPU-thread-bounded.
        Ok(n as u32)
    }
}

// ====================================================================
// SyncTable bridge (R10.2 + R10.4)
// ====================================================================

/// High-byte tag for kernel sys_mutex IDs (RPCS3 C++
/// `lv2_mutex::id_base = 0x85000000`).
pub const MUTEX_ID_BASE: u32 = 0x8500_0000;
/// High-byte tag for kernel sys_semaphore IDs (RPCS3 C++
/// `lv2_sema::id_base = 0x96000000`).
pub const SEMA_ID_BASE: u32 = 0x9600_0000;
/// High-byte tag for kernel sys_cond IDs (RPCS3 C++
/// `lv2_cond::id_base = 0x86000000`).
pub const COND_ID_BASE: u32 = 0x8600_0000;
/// High-byte tag for kernel sys_event_flag IDs (RPCS3 C++
/// `lv2_event_flag::id_base = 0x98000000`).
pub const EVENT_FLAG_ID_BASE: u32 = 0x9800_0000;
/// High-byte tag for kernel sys_rwlock IDs (RPCS3 C++
/// `lv2_rwlock::id_base = 0x88000000`).
pub const RWLOCK_ID_BASE: u32 = 0x8800_0000;
/// High-byte tag for kernel sys_event_queue IDs (RPCS3 C++
/// `lv2_event_queue::id_base = 0x8d000000`).
pub const EVENT_QUEUE_ID_BASE: u32 = 0x8D00_0000;
/// High-byte tag for kernel sys_event_port IDs (RPCS3 C++
/// `lv2_event_port::id_base = 0x0e000000`).
pub const EVENT_PORT_ID_BASE: u32 = 0x0E00_0000;

// Helpers duplicated from `rpcs3-lv2-event-flag` (private there) —
// kept inline so we don't widen that crate's public API just for the
// Lv2SyncState impl. Kept tiny by design: changes there should be
// mirrored here.
#[inline]
fn evf_pattern_matches(pattern: u64, bitptn: u64, mode: u32) -> bool {
    match mode & 0xF {
        WAIT_AND => (pattern & bitptn) == bitptn,
        WAIT_OR => (pattern & bitptn) != 0,
        _ => false,
    }
}
#[inline]
fn evf_apply_clear(pattern: u64, bitptn: u64, mode: u32) -> u64 {
    match mode & !0xF {
        WAIT_CLEAR => pattern & !bitptn,
        WAIT_CLEAR_ALL => 0,
        _ => pattern,
    }
}

const TAG_MASK: u32 = 0x00FF_FFFF;

#[inline]
fn tag(base: u32, raw: u32) -> u32 {
    base | (raw & TAG_MASK)
}
#[inline]
fn untag(id: u32) -> u32 {
    id & TAG_MASK
}

impl SyncTable for Lv2SyncState {
    // -- Mutex ---------------------------------------------------

    fn mutex_create(&mut self, attr: MutexAttr) -> Result<u32, CellError> {
        let id = self.allocate(Lv2SyncKind::Mutex);
        self.mutex_mut(id.raw())?.attr = attr;
        Ok(tag(MUTEX_ID_BASE, id.raw()))
    }

    fn mutex_destroy(&mut self, id: u32) -> Result<(), CellError> {
        let raw = untag(id);
        {
            let m = self.mutex(raw)?;
            if !m.waiters.is_empty() || m.owner.is_some() {
                return Err(CellError::EBUSY);
            }
        }
        self.destroy(raw, Lv2SyncKind::Mutex)
    }

    fn mutex_lock(&mut self, id: u32, tid: u32) -> Result<BlockOutcome, CellError> {
        let m = self.mutex_mut(untag(id))?;
        match m.owner {
            None => {
                m.owner = Some(tid);
                m.recursion_count = 1;
                Ok(BlockOutcome::Acquired)
            }
            Some(owner) if owner == tid => {
                if m.attr.recursive {
                    m.recursion_count = m
                        .recursion_count
                        .checked_add(1)
                        .ok_or(CellError::EKRESOURCE)?;
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
        let m = self.mutex_mut(untag(id))?;
        match m.owner {
            None => {
                m.owner = Some(tid);
                m.recursion_count = 1;
                Ok(())
            }
            Some(owner) if owner == tid && m.attr.recursive => {
                m.recursion_count = m
                    .recursion_count
                    .checked_add(1)
                    .ok_or(CellError::EKRESOURCE)?;
                Ok(())
            }
            _ => Err(CellError::EBUSY),
        }
    }

    fn mutex_unlock(&mut self, id: u32, tid: u32) -> Result<(), CellError> {
        let m = self.mutex_mut(untag(id))?;
        match m.owner {
            Some(owner) if owner == tid => {
                if m.recursion_count == 0 {
                    return Err(CellError::EPERM);
                }
                m.recursion_count -= 1;
                if m.recursion_count == 0 {
                    if let Some(next) = m.waiters.pop_front() {
                        m.owner = Some(next);
                        m.recursion_count = 1;
                    } else {
                        m.owner = None;
                    }
                }
                Ok(())
            }
            _ => Err(CellError::EPERM),
        }
    }

    // -- Semaphore -----------------------------------------------

    fn sema_create(
        &mut self,
        attr: SemaAttr,
        initial: i32,
        max: i32,
    ) -> Result<u32, CellError> {
        let id = self.allocate(Lv2SyncKind::Sema);
        {
            let s = self.sema_mut(id.raw())?;
            s.attr = attr;
            s.value = initial;
            s.max = max;
        }
        Ok(tag(SEMA_ID_BASE, id.raw()))
    }

    fn sema_destroy(&mut self, id: u32) -> Result<(), CellError> {
        let raw = untag(id);
        if !self.sema(raw)?.waiters.is_empty() {
            return Err(CellError::EBUSY);
        }
        self.destroy(raw, Lv2SyncKind::Sema)
    }

    fn sema_post(&mut self, id: u32, count: i32) -> Result<(), CellError> {
        let s = self.sema_mut(untag(id))?;
        // Saturating ceiling check.
        let new_value = s
            .value
            .checked_add(count)
            .ok_or(CellError::EAGAIN)?;
        if new_value > s.max {
            return Err(CellError::EAGAIN);
        }
        s.value = new_value;
        Ok(())
    }

    fn sema_wait(&mut self, id: u32) -> Result<BlockOutcome, CellError> {
        let s = self.sema_mut(untag(id))?;
        if s.value > 0 {
            s.value -= 1;
            Ok(BlockOutcome::Acquired)
        } else {
            Ok(BlockOutcome::MustBlock)
        }
    }

    fn sema_trywait(&mut self, id: u32) -> Result<(), CellError> {
        let s = self.sema_mut(untag(id))?;
        if s.value > 0 {
            s.value -= 1;
            Ok(())
        } else {
            Err(CellError::EBUSY)
        }
    }

    fn sema_get_value(&self, id: u32) -> Result<i32, CellError> {
        Ok(self.sema(untag(id))?.value)
    }
}

// ====================================================================
// CondRegistry bridge (R10.3)
// ====================================================================

impl CondRegistry for Lv2SyncState {
    fn cond_create(&mut self, attr: CondAttr, mutex_id: u32) -> Result<u32, CellError> {
        // The mutex_id passed in is a tagged guest-visible id. Untag
        // it before storing so we can dispatch lookups via mutex_mut.
        let mutex_untagged = untag(mutex_id);
        // Validate the mutex actually exists + is the right kind.
        let _ = self.mutex(mutex_untagged)?;
        let id = self.allocate(Lv2SyncKind::Cond);
        let c = self.cond_mut(id.raw())?;
        c.attr = attr;
        c.mutex_id_untagged = mutex_untagged;
        Ok(tag(COND_ID_BASE, id.raw()))
    }

    fn cond_destroy(&mut self, id: u32) -> Result<(), CellError> {
        let raw = untag(id);
        {
            let c = self.cond(raw)?;
            if !c.waiters.is_empty() || !c.awakened.is_empty() {
                return Err(CellError::EBUSY);
            }
        }
        self.destroy(raw, Lv2SyncKind::Cond)
    }

    fn cond_wait(
        &mut self,
        id: u32,
        tid: u32,
        _timeout_us: u64,
    ) -> Result<WaitOutcome, CellError> {
        let raw = untag(id);
        // Resolve the cv's bound mutex_id first (so we can drop the
        // cond borrow before we mutate the mutex).
        let mutex_untagged = self.cond(raw)?.mutex_id_untagged;
        // Caller must own the mutex.
        {
            let m = self.mutex(mutex_untagged)?;
            if m.owner != Some(tid) {
                return Err(CellError::EPERM);
            }
        }
        // Atomic release+enqueue.
        // Release the mutex; if there are mutex waiters, hand off
        // ownership to the next one (FIFO) — matches mutex_unlock
        // semantics in this crate.
        {
            let m = self.mutex_mut(mutex_untagged)?;
            if let Some(next) = m.waiters.pop_front() {
                m.owner = Some(next);
                m.recursion_count = 1;
            } else {
                m.owner = None;
                m.recursion_count = 0;
            }
        }
        // Enqueue the calling tid on the cv.
        self.cond_mut(raw)?.waiters.push_back(tid);
        Ok(WaitOutcome::MustBlock)
    }

    fn cond_resume_waiter(&mut self, id: u32, tid: u32) -> Result<WaitOutcome, CellError> {
        let raw = untag(id);
        let mutex_untagged = self.cond(raw)?.mutex_id_untagged;
        // The waiter must have been moved to `awakened` by a previous
        // signal/signal_all/signal_to call.
        {
            let c = self.cond_mut(raw)?;
            let idx = c
                .awakened
                .iter()
                .position(|&t| t == tid)
                .ok_or(CellError::EPERM)?;
            c.awakened.remove(idx);
        }
        // Try to take the mutex; if held, park on its waiter queue.
        let m = self.mutex_mut(mutex_untagged)?;
        if m.owner.is_none() {
            m.owner = Some(tid);
            m.recursion_count = 1;
            Ok(WaitOutcome::Woken)
        } else {
            // Stay parked — push to mutex waiter queue if not already there.
            if !m.waiters.iter().any(|&t| t == tid) {
                m.waiters.push_back(tid);
            }
            Ok(WaitOutcome::MustBlock)
        }
    }

    fn cond_signal(&mut self, id: u32) -> Result<Option<u32>, CellError> {
        let c = self.cond_mut(untag(id))?;
        if let Some(t) = c.waiters.pop_front() {
            c.awakened.push_back(t);
            Ok(Some(t))
        } else {
            Ok(None)
        }
    }

    fn cond_signal_all(&mut self, id: u32) -> Result<Vec<u32>, CellError> {
        let c = self.cond_mut(untag(id))?;
        let drained: Vec<u32> = c.waiters.drain(..).collect();
        c.awakened.extend(drained.iter().copied());
        Ok(drained)
    }

    fn cond_signal_to(&mut self, id: u32, tid: u32) -> Result<(), CellError> {
        let c = self.cond_mut(untag(id))?;
        let idx = c
            .waiters
            .iter()
            .position(|&t| t == tid)
            .ok_or(CellError::EPERM)?;
        c.waiters.remove(idx);
        c.awakened.push_back(tid);
        Ok(())
    }
}

// ====================================================================
// EventFlagRegistry bridge (R10.5)
// ====================================================================

impl EventFlagRegistry for Lv2SyncState {
    fn evflag_create(&mut self, attr: EventFlagAttr) -> Result<u32, CellError> {
        let id = self.allocate(Lv2SyncKind::EventFlag);
        let f = self.event_flag_mut(id.raw())?;
        f.attr = attr;
        f.pattern = attr.initial_pattern;
        Ok(tag(EVENT_FLAG_ID_BASE, id.raw()))
    }

    fn evflag_destroy(&mut self, id: u32) -> Result<(), CellError> {
        let raw = untag(id);
        if !self.event_flag(raw)?.waiters.is_empty() {
            return Err(CellError::EBUSY);
        }
        self.destroy(raw, Lv2SyncKind::EventFlag)
    }

    fn evflag_wait(
        &mut self,
        id: u32,
        tid: u32,
        bitptn: u64,
        mode: u32,
        _timeout_us: u64,
    ) -> Result<EvfWaitOutcome, CellError> {
        let f = self.event_flag_mut(untag(id))?;
        // Single-waiter type: refuse if someone is already parked.
        if f.attr.waiter_type == WAITER_SINGLE as i32 && !f.waiters.is_empty() {
            return Err(CellError::EPERM);
        }
        if evf_pattern_matches(f.pattern, bitptn, mode) {
            let snapshot = f.pattern;
            f.pattern = evf_apply_clear(f.pattern, bitptn, mode);
            return Ok(EvfWaitOutcome::Satisfied(snapshot));
        }
        f.waiters.push(EventFlagWaiter { tid, bitptn, mode });
        Ok(EvfWaitOutcome::MustBlock)
    }

    fn evflag_trywait(
        &mut self,
        id: u32,
        bitptn: u64,
        mode: u32,
    ) -> Result<EvfWaitOutcome, CellError> {
        let f = self.event_flag_mut(untag(id))?;
        if evf_pattern_matches(f.pattern, bitptn, mode) {
            let snapshot = f.pattern;
            f.pattern = evf_apply_clear(f.pattern, bitptn, mode);
            Ok(EvfWaitOutcome::Satisfied(snapshot))
        } else {
            Ok(EvfWaitOutcome::NotSatisfied)
        }
    }

    fn evflag_set(&mut self, id: u32, bits: u64) -> Result<Vec<u32>, CellError> {
        let f = self.event_flag_mut(untag(id))?;
        f.pattern |= bits;
        let mut woken = Vec::new();
        let mut i = 0;
        while i < f.waiters.len() {
            let w = f.waiters[i].clone();
            if evf_pattern_matches(f.pattern, w.bitptn, w.mode) {
                f.pattern = evf_apply_clear(f.pattern, w.bitptn, w.mode);
                f.waiters.remove(i);
                woken.push(w.tid);
            } else {
                i += 1;
            }
        }
        Ok(woken)
    }

    fn evflag_clear(&mut self, id: u32, bits: u64) -> Result<(), CellError> {
        let f = self.event_flag_mut(untag(id))?;
        f.pattern &= bits;
        Ok(())
    }

    fn evflag_get(&self, id: u32) -> Result<u64, CellError> {
        Ok(self.event_flag(untag(id))?.pattern)
    }

    fn evflag_cancel(&mut self, id: u32) -> Result<Vec<u32>, CellError> {
        let f = self.event_flag_mut(untag(id))?;
        Ok(f.waiters.drain(..).map(|w| w.tid).collect())
    }
}

// ====================================================================
// RwlockRegistry bridge (R10.7)
// ====================================================================

impl RwlockRegistry for Lv2SyncState {
    fn rwlock_create(&mut self, attr: RwlockAttr) -> Result<u32, CellError> {
        let id = self.allocate(Lv2SyncKind::RwLock);
        self.rwlock_mut(id.raw())?.attr = attr;
        Ok(tag(RWLOCK_ID_BASE, id.raw()))
    }

    fn rwlock_destroy(&mut self, id: u32) -> Result<(), CellError> {
        let raw = untag(id);
        {
            let r = self.rwlock(raw)?;
            if !r.readers.is_empty()
                || r.writer.is_some()
                || !r.read_waiters.is_empty()
                || !r.write_waiters.is_empty()
            {
                return Err(CellError::EBUSY);
            }
        }
        self.destroy(raw, Lv2SyncKind::RwLock)
    }

    fn rwlock_rlock(
        &mut self,
        id: u32,
        tid: u32,
        _timeout_us: u64,
    ) -> Result<RwLockOutcome, CellError> {
        let r = self.rwlock_mut(untag(id))?;
        // Writer-priority: block if a writer holds or any write waiter
        // is queued (so new readers can't starve writers).
        if r.writer.is_some() || !r.write_waiters.is_empty() {
            r.read_waiters.push_back(tid);
            Ok(RwLockOutcome::MustBlock)
        } else {
            r.readers.push(tid);
            Ok(RwLockOutcome::Acquired)
        }
    }

    fn rwlock_tryrlock(
        &mut self,
        id: u32,
        tid: u32,
    ) -> Result<RwLockOutcome, CellError> {
        let r = self.rwlock_mut(untag(id))?;
        if r.writer.is_some() || !r.write_waiters.is_empty() {
            Ok(RwLockOutcome::Busy)
        } else {
            r.readers.push(tid);
            Ok(RwLockOutcome::Acquired)
        }
    }

    fn rwlock_runlock(&mut self, id: u32, tid: u32) -> Result<(), CellError> {
        let r = self.rwlock_mut(untag(id))?;
        let idx = r
            .readers
            .iter()
            .position(|&t| t == tid)
            .ok_or(CellError::EPERM)?;
        r.readers.remove(idx);
        // If we just drained the last reader and a writer is queued,
        // hand the lock to the writer at the head of the queue.
        if r.readers.is_empty() && r.writer.is_none() {
            if let Some(next) = r.write_waiters.pop_front() {
                r.writer = Some(next);
            }
        }
        Ok(())
    }

    fn rwlock_wlock(
        &mut self,
        id: u32,
        tid: u32,
        _timeout_us: u64,
    ) -> Result<RwLockOutcome, CellError> {
        let r = self.rwlock_mut(untag(id))?;
        if r.writer.is_none() && r.readers.is_empty() {
            r.writer = Some(tid);
            Ok(RwLockOutcome::Acquired)
        } else {
            r.write_waiters.push_back(tid);
            Ok(RwLockOutcome::MustBlock)
        }
    }

    fn rwlock_trywlock(
        &mut self,
        id: u32,
        tid: u32,
    ) -> Result<RwLockOutcome, CellError> {
        let r = self.rwlock_mut(untag(id))?;
        if r.writer.is_none() && r.readers.is_empty() {
            r.writer = Some(tid);
            Ok(RwLockOutcome::Acquired)
        } else {
            Ok(RwLockOutcome::Busy)
        }
    }

    fn rwlock_wunlock(&mut self, id: u32, tid: u32) -> Result<(), CellError> {
        let r = self.rwlock_mut(untag(id))?;
        if r.writer != Some(tid) {
            return Err(CellError::EPERM);
        }
        r.writer = None;
        // Hand off: another writer first (writer-priority), else drain
        // all pending readers.
        if let Some(next) = r.write_waiters.pop_front() {
            r.writer = Some(next);
        } else {
            while let Some(reader) = r.read_waiters.pop_front() {
                r.readers.push(reader);
            }
        }
        Ok(())
    }
}

// ====================================================================
// EventRegistry bridge (R10.6)
// ====================================================================

impl EventRegistry for Lv2SyncState {
    fn queue_create(&mut self, attr: QueueAttr, size: u32) -> Result<u32, CellError> {
        let id = self.allocate(Lv2SyncKind::EventQueue);
        let q = self.event_queue_mut(id.raw())?;
        q.attr = attr;
        q.size = size;
        Ok(tag(EVENT_QUEUE_ID_BASE, id.raw()))
    }

    fn queue_destroy(&mut self, id: u32, mode: u32) -> Result<(), CellError> {
        let raw = untag(id);
        {
            let q = self.event_queue(raw)?;
            if !q.pending.is_empty() && mode != QUEUE_DESTROY_FORCE {
                return Err(CellError::EBUSY);
            }
        }
        self.destroy(raw, Lv2SyncKind::EventQueue)
    }

    fn queue_receive(&mut self, id: u32) -> Result<ReceiveOutcome, CellError> {
        let q = self.event_queue_mut(untag(id))?;
        match q.pending.pop_front() {
            Some(ev) => Ok(ReceiveOutcome::Received(ev)),
            None => Ok(ReceiveOutcome::MustBlock),
        }
    }

    fn queue_tryreceive(
        &mut self,
        id: u32,
        max: u32,
    ) -> Result<Vec<Event>, CellError> {
        let q = self.event_queue_mut(untag(id))?;
        let n = (max as usize).min(q.pending.len());
        Ok((0..n).map(|_| q.pending.pop_front().unwrap()).collect())
    }

    fn queue_drain(&mut self, id: u32) -> Result<(), CellError> {
        let q = self.event_queue_mut(untag(id))?;
        q.pending.clear();
        Ok(())
    }

    fn port_create(&mut self, port_type: u32, name: u64) -> Result<u32, CellError> {
        let id = self.allocate(Lv2SyncKind::EventPort);
        let p = self.event_port_mut(id.raw())?;
        p.port_type = port_type;
        p.name = name;
        Ok(tag(EVENT_PORT_ID_BASE, id.raw()))
    }

    fn port_destroy(&mut self, id: u32) -> Result<(), CellError> {
        let raw = untag(id);
        {
            let p = self.event_port(raw)?;
            if p.connected_queue_untagged.is_some() {
                return Err(CellError::EISCONN);
            }
        }
        self.destroy(raw, Lv2SyncKind::EventPort)
    }

    fn port_connect_local(&mut self, port: u32, queue: u32) -> Result<(), CellError> {
        let port_raw = untag(port);
        let queue_raw = untag(queue);
        // Validate queue exists + is correct kind first.
        let _ = self.event_queue(queue_raw)?;
        let p = self.event_port_mut(port_raw)?;
        if p.connected_queue_untagged.is_some() {
            return Err(CellError::EISCONN);
        }
        p.connected_queue_untagged = Some(queue_raw);
        Ok(())
    }

    fn port_disconnect(&mut self, port: u32) -> Result<(), CellError> {
        let p = self.event_port_mut(untag(port))?;
        if p.connected_queue_untagged.is_none() {
            return Err(CellError::ENOTCONN);
        }
        p.connected_queue_untagged = None;
        Ok(())
    }

    fn port_send(
        &mut self,
        port: u32,
        data1: u64,
        data2: u64,
        data3: u64,
    ) -> Result<(), CellError> {
        // Resolve the queue id off the port first, then drop the port
        // borrow before mutating the queue.
        let queue_raw = self
            .event_port(untag(port))?
            .connected_queue_untagged
            .ok_or(CellError::ENOTCONN)?;
        // Source = port id (tagged) — matches C++ behaviour.
        let source = u64::from(port);
        let q = self.event_queue_mut(queue_raw)?;
        if q.pending.len() as u32 >= q.size {
            return Err(CellError::EBUSY);
        }
        q.pending.push_back(Event { source, data1, data2, data3 });
        Ok(())
    }
}

// ====================================================================
// Tests — every R10.1.a acceptance criterion
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- Allocation determinism -----------------------------------

    #[test]
    fn allocate_starts_at_1_and_is_monotonic() {
        let mut s = Lv2SyncState::new();
        let a = s.allocate(Lv2SyncKind::LwMutex);
        let b = s.allocate(Lv2SyncKind::LwMutex);
        let c = s.allocate(Lv2SyncKind::Mutex);
        assert_eq!(a.raw(), 1);
        assert_eq!(b.raw(), 2);
        assert_eq!(c.raw(), 3);
    }

    #[test]
    fn allocate_records_kind_on_handle() {
        let mut s = Lv2SyncState::new();
        let id = s.allocate(Lv2SyncKind::LwMutex);
        assert_eq!(id.kind(), Lv2SyncKind::LwMutex);
    }

    #[test]
    fn allocate_does_not_reuse_after_destroy() {
        let mut s = Lv2SyncState::new();
        let a = s.allocate(Lv2SyncKind::LwMutex);
        s.destroy(a.raw(), Lv2SyncKind::LwMutex).unwrap();
        let b = s.allocate(Lv2SyncKind::LwMutex);
        assert_eq!(b.raw(), 2, "ids must not be recycled");
    }

    // -- Lookup by id ----------------------------------------------

    #[test]
    fn lw_mutex_lookup_returns_default_state() {
        let mut s = Lv2SyncState::new();
        let id = s.allocate(Lv2SyncKind::LwMutex);
        let m = s.lw_mutex(id.raw()).expect("looks up fine");
        assert!(m.owner.is_none());
        assert_eq!(m.recursion_count, 0);
        assert!(m.waiters.is_empty());
    }

    #[test]
    fn lw_mutex_unknown_id_is_esrch() {
        let s = Lv2SyncState::new();
        assert_eq!(s.lw_mutex(999), Err(CellError::ESRCH));
    }

    #[test]
    fn lw_mutex_mut_returns_same_default() {
        let mut s = Lv2SyncState::new();
        let id = s.allocate(Lv2SyncKind::LwMutex);
        let m = s.lw_mutex_mut(id.raw()).unwrap();
        m.owner = Some(42);
        m.recursion_count = 1;
        let m = s.lw_mutex(id.raw()).unwrap();
        assert_eq!(m.owner, Some(42));
        assert_eq!(m.recursion_count, 1);
    }

    // -- Wrong-kind rejection --------------------------------------

    #[test]
    fn lw_mutex_lookup_on_sema_handle_is_einval() {
        let mut s = Lv2SyncState::new();
        let id = s.allocate(Lv2SyncKind::Sema);
        assert_eq!(s.lw_mutex(id.raw()), Err(CellError::EINVAL));
    }

    #[test]
    fn destroy_with_wrong_kind_is_einval() {
        let mut s = Lv2SyncState::new();
        let id = s.allocate(Lv2SyncKind::LwMutex);
        // Caller claims it's a sema; should refuse.
        assert_eq!(s.destroy(id.raw(), Lv2SyncKind::Sema), Err(CellError::EINVAL));
        // And the handle should still be alive afterward.
        assert!(s.lw_mutex(id.raw()).is_ok());
    }

    // -- Double destroy / unknown destroy -------------------------

    #[test]
    fn destroy_unknown_is_esrch() {
        let mut s = Lv2SyncState::new();
        assert_eq!(
            s.destroy(7, Lv2SyncKind::LwMutex),
            Err(CellError::ESRCH)
        );
    }

    #[test]
    fn destroy_twice_is_esrch_on_second_call() {
        let mut s = Lv2SyncState::new();
        let id = s.allocate(Lv2SyncKind::LwMutex);
        s.destroy(id.raw(), Lv2SyncKind::LwMutex).unwrap();
        assert_eq!(
            s.destroy(id.raw(), Lv2SyncKind::LwMutex),
            Err(CellError::ESRCH),
        );
    }

    // -- Isolation between instances -------------------------------

    #[test]
    fn two_states_do_not_share_ids() {
        let mut a = Lv2SyncState::new();
        let mut b = Lv2SyncState::new();
        let ida = a.allocate(Lv2SyncKind::LwMutex);
        let idb = b.allocate(Lv2SyncKind::LwMutex);
        // Both start at 1 because counters are independent.
        assert_eq!(ida.raw(), 1);
        assert_eq!(idb.raw(), 1);
        // And a's handle is not visible to b.
        assert_eq!(b.lw_mutex(ida.raw()).map(|_| ()), b.lw_mutex(1).map(|_| ()));
        // Mutating one must not touch the other.
        a.lw_mutex_mut(ida.raw()).unwrap().owner = Some(7);
        assert_eq!(b.lw_mutex(idb.raw()).unwrap().owner, None);
    }

    // -- LwMutex object roundtrip ---------------------------------

    #[test]
    fn lwmutex_alloc_and_destroy_round_trip() {
        let mut s = Lv2SyncState::new();
        assert_eq!(s.len(), 0);
        assert!(s.is_empty());
        let id = s.allocate(Lv2SyncKind::LwMutex);
        assert_eq!(s.len(), 1);
        assert!(!s.is_empty());
        s.destroy(id.raw(), Lv2SyncKind::LwMutex).unwrap();
        assert_eq!(s.len(), 0);
        assert!(s.is_empty());
    }

    // -- LwMutexTable impl (R10.1.b) ------------------------------

    #[test]
    fn lwmutex_table_create_returns_tagged_id() {
        let mut s = Lv2SyncState::new();
        let id = s.lwmutex_create(0x01).unwrap();
        assert_eq!(id & 0xFF00_0000, LWMUTEX_ID_BASE);
        assert_eq!(id & LWMUTEX_ID_MASK, 1); // first allocation
    }

    #[test]
    fn lwmutex_table_create_records_protocol_in_attr() {
        let mut s = Lv2SyncState::new();
        let id = s.lwmutex_create(0x02).unwrap();
        let raw = Lv2SyncState::lwmutex_untag(id);
        assert_eq!(s.lw_mutex(raw).unwrap().attr.protocol, 0x02);
    }

    #[test]
    fn lwmutex_table_enqueue_dequeue_is_fifo() {
        let mut s = Lv2SyncState::new();
        let id = s.lwmutex_create(0x01).unwrap();
        s.lwmutex_enqueue(id, 10).unwrap();
        s.lwmutex_enqueue(id, 11).unwrap();
        s.lwmutex_enqueue(id, 12).unwrap();
        assert_eq!(s.lwmutex_waiter_count(id), Ok(3));
        assert_eq!(s.lwmutex_dequeue(id), Ok(Some(10)));
        assert_eq!(s.lwmutex_dequeue(id), Ok(Some(11)));
        assert_eq!(s.lwmutex_dequeue(id), Ok(Some(12)));
        assert_eq!(s.lwmutex_dequeue(id), Ok(None));
        assert_eq!(s.lwmutex_waiter_count(id), Ok(0));
    }

    #[test]
    fn lwmutex_table_destroy_with_waiters_is_ebusy() {
        let mut s = Lv2SyncState::new();
        let id = s.lwmutex_create(0x01).unwrap();
        s.lwmutex_enqueue(id, 42).unwrap();
        assert_eq!(s.lwmutex_destroy(id), Err(CellError::EBUSY));
        // Drain and retry — destroy now succeeds.
        s.lwmutex_dequeue(id).unwrap();
        assert_eq!(s.lwmutex_destroy(id), Ok(()));
    }

    #[test]
    fn lwmutex_table_unknown_id_is_esrch() {
        let mut s = Lv2SyncState::new();
        // Tagged id whose lower bits don't correspond to any entry.
        assert_eq!(
            s.lwmutex_destroy(LWMUTEX_ID_BASE | 0xFF),
            Err(CellError::ESRCH)
        );
        assert_eq!(
            s.lwmutex_enqueue(LWMUTEX_ID_BASE | 0xFF, 7),
            Err(CellError::ESRCH)
        );
        assert_eq!(
            s.lwmutex_waiter_count(LWMUTEX_ID_BASE | 0xFF),
            Err(CellError::ESRCH)
        );
    }

    #[test]
    fn lwmutex_table_destroy_then_reuse_id_returns_esrch() {
        let mut s = Lv2SyncState::new();
        let id = s.lwmutex_create(0x01).unwrap();
        s.lwmutex_destroy(id).unwrap();
        assert_eq!(s.lwmutex_destroy(id), Err(CellError::ESRCH));
    }

    // -- SyncTable mutex impl (R10.2) ------------------------------

    #[test]
    fn synctable_mutex_create_returns_tagged_id() {
        let mut s = Lv2SyncState::new();
        let id = SyncTable::mutex_create(&mut s, MutexAttr::default()).unwrap();
        assert_eq!(id & 0xFF00_0000, MUTEX_ID_BASE);
        assert_eq!(id & TAG_MASK, 1);
    }

    #[test]
    fn synctable_mutex_lock_acquires_when_free() {
        let mut s = Lv2SyncState::new();
        let id = SyncTable::mutex_create(&mut s, MutexAttr::default()).unwrap();
        assert_eq!(s.mutex_lock(id, 100), Ok(BlockOutcome::Acquired));
        assert_eq!(s.mutex(untag(id)).unwrap().owner, Some(100));
    }

    #[test]
    fn synctable_mutex_lock_blocks_on_contention() {
        let mut s = Lv2SyncState::new();
        let id = SyncTable::mutex_create(&mut s, MutexAttr::default()).unwrap();
        s.mutex_lock(id, 100).unwrap();
        assert_eq!(s.mutex_lock(id, 200), Ok(BlockOutcome::MustBlock));
        assert_eq!(s.mutex(untag(id)).unwrap().waiters, vec![200u32]);
    }

    #[test]
    fn synctable_mutex_self_lock_non_recursive_is_edeadlk() {
        let mut s = Lv2SyncState::new();
        let id = SyncTable::mutex_create(&mut s, MutexAttr::default()).unwrap();
        s.mutex_lock(id, 100).unwrap();
        assert_eq!(s.mutex_lock(id, 100), Err(CellError::EDEADLK));
    }

    #[test]
    fn synctable_mutex_self_lock_recursive_increments() {
        let mut s = Lv2SyncState::new();
        let attr = MutexAttr { protocol: 1, recursive: true };
        let id = SyncTable::mutex_create(&mut s, attr).unwrap();
        s.mutex_lock(id, 100).unwrap();
        s.mutex_lock(id, 100).unwrap();
        assert_eq!(s.mutex(untag(id)).unwrap().recursion_count, 2);
    }

    #[test]
    fn synctable_mutex_unlock_hands_off_to_next_waiter() {
        let mut s = Lv2SyncState::new();
        let id = SyncTable::mutex_create(&mut s, MutexAttr::default()).unwrap();
        s.mutex_lock(id, 100).unwrap(); // tid 100 holds
        s.mutex_lock(id, 200).unwrap(); // tid 200 parks
        s.mutex_lock(id, 300).unwrap(); // tid 300 parks
        s.mutex_unlock(id, 100).unwrap();
        // 200 is now the owner; 300 still waits.
        let m = s.mutex(untag(id)).unwrap();
        assert_eq!(m.owner, Some(200));
        assert_eq!(m.waiters, vec![300u32]);
    }

    #[test]
    fn synctable_mutex_unlock_not_owner_is_eperm() {
        let mut s = Lv2SyncState::new();
        let id = SyncTable::mutex_create(&mut s, MutexAttr::default()).unwrap();
        s.mutex_lock(id, 100).unwrap();
        assert_eq!(s.mutex_unlock(id, 200), Err(CellError::EPERM));
    }

    #[test]
    fn synctable_mutex_trylock_busy_when_held() {
        let mut s = Lv2SyncState::new();
        let id = SyncTable::mutex_create(&mut s, MutexAttr::default()).unwrap();
        s.mutex_lock(id, 100).unwrap();
        assert_eq!(s.mutex_trylock(id, 200), Err(CellError::EBUSY));
    }

    #[test]
    fn synctable_mutex_destroy_held_is_ebusy() {
        let mut s = Lv2SyncState::new();
        let id = SyncTable::mutex_create(&mut s, MutexAttr::default()).unwrap();
        s.mutex_lock(id, 100).unwrap();
        assert_eq!(s.mutex_destroy(id), Err(CellError::EBUSY));
    }

    #[test]
    fn synctable_mutex_destroy_when_free() {
        let mut s = Lv2SyncState::new();
        let id = SyncTable::mutex_create(&mut s, MutexAttr::default()).unwrap();
        s.mutex_lock(id, 100).unwrap();
        s.mutex_unlock(id, 100).unwrap();
        assert_eq!(s.mutex_destroy(id), Ok(()));
    }

    // -- SyncTable sema impl (R10.4) -------------------------------

    #[test]
    fn synctable_sema_create_returns_tagged_id() {
        let mut s = Lv2SyncState::new();
        let id = SyncTable::sema_create(&mut s, SemaAttr::default(), 3, 10).unwrap();
        assert_eq!(id & 0xFF00_0000, SEMA_ID_BASE);
    }

    #[test]
    fn synctable_sema_initial_value_round_trips() {
        let mut s = Lv2SyncState::new();
        let id = SyncTable::sema_create(&mut s, SemaAttr::default(), 7, 10).unwrap();
        assert_eq!(s.sema_get_value(id), Ok(7));
    }

    #[test]
    fn synctable_sema_post_increments() {
        let mut s = Lv2SyncState::new();
        let id = SyncTable::sema_create(&mut s, SemaAttr::default(), 0, 10).unwrap();
        s.sema_post(id, 3).unwrap();
        assert_eq!(s.sema_get_value(id), Ok(3));
    }

    #[test]
    fn synctable_sema_post_overflow_is_eagain() {
        let mut s = Lv2SyncState::new();
        let id = SyncTable::sema_create(&mut s, SemaAttr::default(), 5, 5).unwrap();
        assert_eq!(s.sema_post(id, 1), Err(CellError::EAGAIN));
    }

    #[test]
    fn synctable_sema_wait_blocks_at_zero() {
        let mut s = Lv2SyncState::new();
        let id = SyncTable::sema_create(&mut s, SemaAttr::default(), 0, 10).unwrap();
        assert_eq!(s.sema_wait(id), Ok(BlockOutcome::MustBlock));
    }

    #[test]
    fn synctable_sema_wait_decrements_when_positive() {
        let mut s = Lv2SyncState::new();
        let id = SyncTable::sema_create(&mut s, SemaAttr::default(), 2, 10).unwrap();
        assert_eq!(s.sema_wait(id), Ok(BlockOutcome::Acquired));
        assert_eq!(s.sema_get_value(id), Ok(1));
    }

    #[test]
    fn synctable_sema_trywait_busy_at_zero() {
        let mut s = Lv2SyncState::new();
        let id = SyncTable::sema_create(&mut s, SemaAttr::default(), 0, 10).unwrap();
        assert_eq!(s.sema_trywait(id), Err(CellError::EBUSY));
    }

    #[test]
    fn synctable_mutex_and_sema_have_distinct_id_spaces() {
        let mut s = Lv2SyncState::new();
        let m = SyncTable::mutex_create(&mut s, MutexAttr::default()).unwrap();
        let q = SyncTable::sema_create(&mut s, SemaAttr::default(), 0, 10).unwrap();
        assert_ne!(m & 0xFF00_0000, q & 0xFF00_0000);
        // And lookups via the wrong kind fail with EINVAL.
        assert_eq!(s.sema(untag(m)), Err(CellError::EINVAL));
        assert_eq!(s.mutex(untag(q)), Err(CellError::EINVAL));
    }

    // -- CondRegistry impl (R10.3) --------------------------------

    fn setup_cond() -> (Lv2SyncState, u32, u32) {
        // Returns (state, mutex_id, cond_id) for tests that need a
        // bound (mutex, cv) pair.
        let mut s = Lv2SyncState::new();
        let m = SyncTable::mutex_create(&mut s, MutexAttr::default()).unwrap();
        let c = CondRegistry::cond_create(&mut s, CondAttr::default(), m).unwrap();
        (s, m, c)
    }

    #[test]
    fn condreg_create_returns_tagged_id() {
        let (_s, _m, c) = setup_cond();
        assert_eq!(c & 0xFF00_0000, COND_ID_BASE);
    }

    #[test]
    fn condreg_create_rejects_unknown_mutex() {
        let mut s = Lv2SyncState::new();
        let bogus_mutex = MUTEX_ID_BASE | 0xFE;
        assert_eq!(
            CondRegistry::cond_create(&mut s, CondAttr::default(), bogus_mutex),
            Err(CellError::ESRCH)
        );
    }

    #[test]
    fn condreg_wait_requires_caller_to_own_mutex() {
        let (mut s, m, c) = setup_cond();
        // Nobody has acquired the mutex yet → cond_wait → EPERM.
        assert_eq!(
            s.cond_wait(c, 100, 0),
            Err(CellError::EPERM)
        );
        // Acquire the mutex; now cond_wait must succeed and park.
        let _ = m;
        s.mutex_lock(m, 100).unwrap();
        assert_eq!(s.cond_wait(c, 100, 0), Ok(WaitOutcome::MustBlock));
        // Mutex was released by cond_wait.
        assert_eq!(s.mutex(untag(m)).unwrap().owner, None);
    }

    #[test]
    fn condreg_signal_moves_waiter_to_awakened() {
        let (mut s, m, c) = setup_cond();
        s.mutex_lock(m, 100).unwrap();
        s.cond_wait(c, 100, 0).unwrap();
        assert_eq!(s.cond_signal(c), Ok(Some(100)));
        // Now in `awakened`, not `waiters`.
        let cond = s.cond(untag(c)).unwrap();
        assert!(cond.waiters.is_empty());
        assert_eq!(cond.awakened, vec![100u32]);
    }

    #[test]
    fn condreg_signal_empty_queue_returns_none() {
        let (mut s, _m, c) = setup_cond();
        assert_eq!(s.cond_signal(c), Ok(None));
    }

    #[test]
    fn condreg_signal_all_drains_to_awakened() {
        let (mut s, m, c) = setup_cond();
        // tid 100 waits.
        s.mutex_lock(m, 100).unwrap();
        s.cond_wait(c, 100, 0).unwrap();
        // tid 200 also waits (after 100 released the mutex through wait).
        s.mutex_lock(m, 200).unwrap();
        s.cond_wait(c, 200, 0).unwrap();
        let woken = s.cond_signal_all(c).unwrap();
        assert_eq!(woken, vec![100, 200]);
        let cond = s.cond(untag(c)).unwrap();
        assert!(cond.waiters.is_empty());
        assert_eq!(cond.awakened.len(), 2);
    }

    #[test]
    fn condreg_signal_to_specific_thread() {
        let (mut s, m, c) = setup_cond();
        s.mutex_lock(m, 100).unwrap();
        s.cond_wait(c, 100, 0).unwrap();
        s.mutex_lock(m, 200).unwrap();
        s.cond_wait(c, 200, 0).unwrap();
        assert_eq!(s.cond_signal_to(c, 200), Ok(()));
        let cond = s.cond(untag(c)).unwrap();
        // 100 stays in waiters; 200 moves to awakened.
        assert_eq!(cond.waiters, vec![100u32]);
        assert_eq!(cond.awakened, vec![200u32]);
    }

    #[test]
    fn condreg_signal_to_unparked_thread_is_eperm() {
        let (mut s, _m, c) = setup_cond();
        assert_eq!(s.cond_signal_to(c, 999), Err(CellError::EPERM));
    }

    #[test]
    fn condreg_resume_waiter_acquires_free_mutex() {
        let (mut s, m, c) = setup_cond();
        s.mutex_lock(m, 100).unwrap();
        s.cond_wait(c, 100, 0).unwrap();
        // After cond_wait, mutex is free.
        s.cond_signal(c).unwrap();
        // Resume: mutex is free → acquire + return Woken.
        assert_eq!(s.cond_resume_waiter(c, 100), Ok(WaitOutcome::Woken));
        assert_eq!(s.mutex(untag(m)).unwrap().owner, Some(100));
    }

    #[test]
    fn condreg_resume_waiter_parks_on_held_mutex() {
        let (mut s, m, c) = setup_cond();
        s.mutex_lock(m, 100).unwrap();
        s.cond_wait(c, 100, 0).unwrap();
        // Some other thread grabs the mutex before 100 resumes.
        s.mutex_lock(m, 200).unwrap();
        s.cond_signal(c).unwrap();
        // 100 wakes but mutex is held → must park.
        assert_eq!(s.cond_resume_waiter(c, 100), Ok(WaitOutcome::MustBlock));
        // And 100 should be in the mutex waiters now.
        assert_eq!(s.mutex(untag(m)).unwrap().waiters, vec![100u32]);
    }

    #[test]
    fn condreg_destroy_with_waiters_is_ebusy() {
        let (mut s, m, c) = setup_cond();
        s.mutex_lock(m, 100).unwrap();
        s.cond_wait(c, 100, 0).unwrap();
        assert_eq!(s.cond_destroy(c), Err(CellError::EBUSY));
    }

    #[test]
    fn condreg_destroy_when_empty() {
        let (mut s, _m, c) = setup_cond();
        assert_eq!(s.cond_destroy(c), Ok(()));
    }

    // -- EventFlagRegistry impl (R10.5) ----------------------------

    fn evf_attr(initial: u64) -> EventFlagAttr {
        EventFlagAttr { initial_pattern: initial, ..EventFlagAttr::default() }
    }

    #[test]
    fn evfreg_create_returns_tagged_id() {
        let mut s = Lv2SyncState::new();
        let id =
            EventFlagRegistry::evflag_create(&mut s, evf_attr(0)).unwrap();
        assert_eq!(id & 0xFF00_0000, EVENT_FLAG_ID_BASE);
    }

    #[test]
    fn evfreg_initial_pattern_round_trips() {
        let mut s = Lv2SyncState::new();
        let id =
            EventFlagRegistry::evflag_create(&mut s, evf_attr(0xDEAD_BEEF))
                .unwrap();
        assert_eq!(s.evflag_get(id), Ok(0xDEAD_BEEF));
    }

    #[test]
    fn evfreg_trywait_and_with_matching_pattern_clears_when_requested() {
        let mut s = Lv2SyncState::new();
        let id =
            EventFlagRegistry::evflag_create(&mut s, evf_attr(0xFF)).unwrap();
        // WAIT_AND with bitptn=0x0F: requires all bits set.
        let res = s.evflag_trywait(id, 0x0F, WAIT_AND | WAIT_CLEAR);
        assert_eq!(res, Ok(EvfWaitOutcome::Satisfied(0xFF)));
        // After clear, pattern is 0xF0.
        assert_eq!(s.evflag_get(id), Ok(0xF0));
    }

    #[test]
    fn evfreg_trywait_or_matches_any_bit() {
        let mut s = Lv2SyncState::new();
        let id =
            EventFlagRegistry::evflag_create(&mut s, evf_attr(0x10)).unwrap();
        let res = s.evflag_trywait(id, 0xFF, WAIT_OR);
        assert_eq!(res, Ok(EvfWaitOutcome::Satisfied(0x10)));
    }

    #[test]
    fn evfreg_trywait_not_satisfied_returns_not_satisfied() {
        let mut s = Lv2SyncState::new();
        let id =
            EventFlagRegistry::evflag_create(&mut s, evf_attr(0)).unwrap();
        assert_eq!(
            s.evflag_trywait(id, 0xFF, WAIT_AND),
            Ok(EvfWaitOutcome::NotSatisfied)
        );
    }

    #[test]
    fn evfreg_wait_parks_when_pattern_does_not_match() {
        let mut s = Lv2SyncState::new();
        let id =
            EventFlagRegistry::evflag_create(&mut s, evf_attr(0)).unwrap();
        let res = s.evflag_wait(id, 100, 0x01, WAIT_AND, 0);
        assert_eq!(res, Ok(EvfWaitOutcome::MustBlock));
        // Waiter is recorded.
        assert_eq!(s.event_flag(untag(id)).unwrap().waiters.len(), 1);
    }

    #[test]
    fn evfreg_set_wakes_matching_waiter() {
        let mut s = Lv2SyncState::new();
        let id =
            EventFlagRegistry::evflag_create(&mut s, evf_attr(0)).unwrap();
        s.evflag_wait(id, 100, 0x01, WAIT_AND, 0).unwrap();
        let woken = s.evflag_set(id, 0x01).unwrap();
        assert_eq!(woken, vec![100u32]);
        // Default mode had no CLEAR; pattern stays 0x01.
        assert_eq!(s.evflag_get(id), Ok(0x01));
    }

    #[test]
    fn evfreg_set_with_clear_drops_matched_bits_after_wake() {
        let mut s = Lv2SyncState::new();
        let id =
            EventFlagRegistry::evflag_create(&mut s, evf_attr(0)).unwrap();
        s.evflag_wait(id, 100, 0x01, WAIT_AND | WAIT_CLEAR, 0)
            .unwrap();
        let woken = s.evflag_set(id, 0x03).unwrap();
        assert_eq!(woken, vec![100u32]);
        // WAIT_CLEAR took out the matched bit; 0x03 & !0x01 = 0x02.
        assert_eq!(s.evflag_get(id), Ok(0x02));
    }

    #[test]
    fn evfreg_clear_masks_pattern() {
        let mut s = Lv2SyncState::new();
        let id =
            EventFlagRegistry::evflag_create(&mut s, evf_attr(0xFF)).unwrap();
        s.evflag_clear(id, 0x0F).unwrap();
        assert_eq!(s.evflag_get(id), Ok(0x0F));
    }

    #[test]
    fn evfreg_cancel_returns_all_waiters() {
        let mut s = Lv2SyncState::new();
        let id =
            EventFlagRegistry::evflag_create(&mut s, evf_attr(0)).unwrap();
        s.evflag_wait(id, 100, 0x01, WAIT_AND, 0).unwrap();
        s.evflag_wait(id, 200, 0x02, WAIT_AND, 0).unwrap();
        let cancelled = s.evflag_cancel(id).unwrap();
        assert_eq!(cancelled, vec![100u32, 200u32]);
        assert!(s.event_flag(untag(id)).unwrap().waiters.is_empty());
    }

    #[test]
    fn evfreg_destroy_with_waiters_is_ebusy() {
        let mut s = Lv2SyncState::new();
        let id =
            EventFlagRegistry::evflag_create(&mut s, evf_attr(0)).unwrap();
        s.evflag_wait(id, 100, 0x01, WAIT_AND, 0).unwrap();
        assert_eq!(s.evflag_destroy(id), Err(CellError::EBUSY));
    }

    #[test]
    fn evfreg_destroy_when_empty() {
        let mut s = Lv2SyncState::new();
        let id =
            EventFlagRegistry::evflag_create(&mut s, evf_attr(0)).unwrap();
        assert_eq!(s.evflag_destroy(id), Ok(()));
    }

    // -- RwlockRegistry impl (R10.7) -------------------------------

    fn make_rwlock(s: &mut Lv2SyncState) -> u32 {
        RwlockRegistry::rwlock_create(s, RwlockAttr::default()).unwrap()
    }

    #[test]
    fn rwlockreg_create_returns_tagged_id() {
        let mut s = Lv2SyncState::new();
        let id = make_rwlock(&mut s);
        assert_eq!(id & 0xFF00_0000, RWLOCK_ID_BASE);
    }

    #[test]
    fn rwlockreg_rlock_acquires_when_free() {
        let mut s = Lv2SyncState::new();
        let id = make_rwlock(&mut s);
        assert_eq!(s.rwlock_rlock(id, 100, 0), Ok(RwLockOutcome::Acquired));
        assert_eq!(s.rwlock(untag(id)).unwrap().readers, vec![100u32]);
    }

    #[test]
    fn rwlockreg_two_readers_can_share() {
        let mut s = Lv2SyncState::new();
        let id = make_rwlock(&mut s);
        s.rwlock_rlock(id, 100, 0).unwrap();
        assert_eq!(s.rwlock_rlock(id, 200, 0), Ok(RwLockOutcome::Acquired));
        assert_eq!(s.rwlock(untag(id)).unwrap().readers, vec![100u32, 200u32]);
    }

    #[test]
    fn rwlockreg_wlock_blocks_with_readers() {
        let mut s = Lv2SyncState::new();
        let id = make_rwlock(&mut s);
        s.rwlock_rlock(id, 100, 0).unwrap();
        assert_eq!(s.rwlock_wlock(id, 200, 0), Ok(RwLockOutcome::MustBlock));
        assert_eq!(s.rwlock(untag(id)).unwrap().write_waiters.len(), 1);
    }

    #[test]
    fn rwlockreg_writer_priority_blocks_new_readers() {
        let mut s = Lv2SyncState::new();
        let id = make_rwlock(&mut s);
        s.rwlock_rlock(id, 100, 0).unwrap();
        // Writer queues up.
        s.rwlock_wlock(id, 200, 0).unwrap(); // MustBlock
        // New reader must NOT slip in — writer-priority.
        assert_eq!(s.rwlock_rlock(id, 300, 0), Ok(RwLockOutcome::MustBlock));
    }

    #[test]
    fn rwlockreg_runlock_drains_to_writer() {
        let mut s = Lv2SyncState::new();
        let id = make_rwlock(&mut s);
        s.rwlock_rlock(id, 100, 0).unwrap();
        s.rwlock_wlock(id, 200, 0).unwrap();
        s.rwlock_runlock(id, 100).unwrap();
        let r = s.rwlock(untag(id)).unwrap();
        assert!(r.readers.is_empty());
        assert_eq!(r.writer, Some(200));
        assert!(r.write_waiters.is_empty());
    }

    #[test]
    fn rwlockreg_runlock_not_holder_is_eperm() {
        let mut s = Lv2SyncState::new();
        let id = make_rwlock(&mut s);
        s.rwlock_rlock(id, 100, 0).unwrap();
        assert_eq!(s.rwlock_runlock(id, 200), Err(CellError::EPERM));
    }

    #[test]
    fn rwlockreg_wunlock_hands_off_to_next_writer() {
        let mut s = Lv2SyncState::new();
        let id = make_rwlock(&mut s);
        s.rwlock_wlock(id, 100, 0).unwrap(); // 100 holds
        s.rwlock_wlock(id, 200, 0).unwrap(); // 200 queues
        s.rwlock_wlock(id, 300, 0).unwrap(); // 300 queues
        s.rwlock_wunlock(id, 100).unwrap();
        let r = s.rwlock(untag(id)).unwrap();
        assert_eq!(r.writer, Some(200));
        assert_eq!(r.write_waiters.len(), 1);
    }

    #[test]
    fn rwlockreg_wunlock_drains_all_readers_when_no_writer_queued() {
        let mut s = Lv2SyncState::new();
        let id = make_rwlock(&mut s);
        s.rwlock_wlock(id, 100, 0).unwrap();
        // Readers queue (blocked by the writer).
        s.rwlock_rlock(id, 200, 0).unwrap();
        s.rwlock_rlock(id, 300, 0).unwrap();
        s.rwlock_wunlock(id, 100).unwrap();
        let r = s.rwlock(untag(id)).unwrap();
        assert_eq!(r.writer, None);
        assert_eq!(r.readers, vec![200u32, 300u32]);
        assert!(r.read_waiters.is_empty());
    }

    #[test]
    fn rwlockreg_wunlock_not_holder_is_eperm() {
        let mut s = Lv2SyncState::new();
        let id = make_rwlock(&mut s);
        s.rwlock_wlock(id, 100, 0).unwrap();
        assert_eq!(s.rwlock_wunlock(id, 200), Err(CellError::EPERM));
    }

    #[test]
    fn rwlockreg_try_paths_match_lock_paths() {
        let mut s = Lv2SyncState::new();
        let id = make_rwlock(&mut s);
        assert_eq!(s.rwlock_tryrlock(id, 100), Ok(RwLockOutcome::Acquired));
        assert_eq!(s.rwlock_trywlock(id, 200), Ok(RwLockOutcome::Busy));
        s.rwlock_runlock(id, 100).unwrap();
        assert_eq!(s.rwlock_trywlock(id, 200), Ok(RwLockOutcome::Acquired));
    }

    #[test]
    fn rwlockreg_destroy_held_is_ebusy() {
        let mut s = Lv2SyncState::new();
        let id = make_rwlock(&mut s);
        s.rwlock_wlock(id, 100, 0).unwrap();
        assert_eq!(s.rwlock_destroy(id), Err(CellError::EBUSY));
    }

    #[test]
    fn rwlockreg_destroy_when_free() {
        let mut s = Lv2SyncState::new();
        let id = make_rwlock(&mut s);
        assert_eq!(s.rwlock_destroy(id), Ok(()));
    }

    // -- EventRegistry impl (R10.6) --------------------------------

    fn setup_event() -> (Lv2SyncState, u32, u32) {
        // Returns (state, queue_id, port_id) bound to each other.
        let mut s = Lv2SyncState::new();
        let q = EventRegistry::queue_create(&mut s, QueueAttr::default(), 8)
            .unwrap();
        let p = EventRegistry::port_create(&mut s, 1, 0).unwrap();
        s.port_connect_local(p, q).unwrap();
        (s, q, p)
    }

    #[test]
    fn evreg_queue_create_returns_tagged_id() {
        let mut s = Lv2SyncState::new();
        let q =
            EventRegistry::queue_create(&mut s, QueueAttr::default(), 8).unwrap();
        assert_eq!(q & 0xFF00_0000, EVENT_QUEUE_ID_BASE);
    }

    #[test]
    fn evreg_port_create_returns_tagged_id() {
        let mut s = Lv2SyncState::new();
        let p = EventRegistry::port_create(&mut s, 1, 0).unwrap();
        assert_eq!(p & 0xFF00_0000, EVENT_PORT_ID_BASE);
    }

    #[test]
    fn evreg_receive_empty_queue_returns_must_block() {
        let mut s = Lv2SyncState::new();
        let q =
            EventRegistry::queue_create(&mut s, QueueAttr::default(), 8).unwrap();
        assert_eq!(s.queue_receive(q), Ok(ReceiveOutcome::MustBlock));
    }

    #[test]
    fn evreg_send_then_receive_round_trips() {
        let (mut s, q, p) = setup_event();
        s.port_send(p, 0xAA, 0xBB, 0xCC).unwrap();
        let outcome = s.queue_receive(q).unwrap();
        match outcome {
            ReceiveOutcome::Received(ev) => {
                assert_eq!(ev.source, u64::from(p));
                assert_eq!(ev.data1, 0xAA);
                assert_eq!(ev.data2, 0xBB);
                assert_eq!(ev.data3, 0xCC);
            }
            _ => panic!("expected Received"),
        }
    }

    #[test]
    fn evreg_send_full_queue_is_ebusy() {
        let mut s = Lv2SyncState::new();
        let q = EventRegistry::queue_create(&mut s, QueueAttr::default(), 1)
            .unwrap();
        let p = EventRegistry::port_create(&mut s, 1, 0).unwrap();
        s.port_connect_local(p, q).unwrap();
        s.port_send(p, 0, 0, 0).unwrap();
        assert_eq!(s.port_send(p, 0, 0, 0), Err(CellError::EBUSY));
    }

    #[test]
    fn evreg_tryreceive_drains_up_to_max() {
        let (mut s, q, p) = setup_event();
        s.port_send(p, 1, 0, 0).unwrap();
        s.port_send(p, 2, 0, 0).unwrap();
        s.port_send(p, 3, 0, 0).unwrap();
        let drained = s.queue_tryreceive(q, 2).unwrap();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].data1, 1);
        assert_eq!(drained[1].data1, 2);
    }

    #[test]
    fn evreg_drain_clears_queue() {
        let (mut s, q, p) = setup_event();
        s.port_send(p, 0, 0, 0).unwrap();
        s.port_send(p, 0, 0, 0).unwrap();
        s.queue_drain(q).unwrap();
        assert_eq!(s.event_queue(untag(q)).unwrap().pending.len(), 0);
    }

    #[test]
    fn evreg_port_disconnect_then_connect_again() {
        let (mut s, q, p) = setup_event();
        s.port_disconnect(p).unwrap();
        // Second disconnect → ENOTCONN.
        assert_eq!(s.port_disconnect(p), Err(CellError::ENOTCONN));
        // Reconnect works.
        s.port_connect_local(p, q).unwrap();
        // Connecting twice without disconnect → EISCONN.
        assert_eq!(s.port_connect_local(p, q), Err(CellError::EISCONN));
    }

    #[test]
    fn evreg_port_send_disconnected_is_enotconn() {
        let mut s = Lv2SyncState::new();
        let p = EventRegistry::port_create(&mut s, 1, 0).unwrap();
        assert_eq!(s.port_send(p, 0, 0, 0), Err(CellError::ENOTCONN));
    }

    #[test]
    fn evreg_port_destroy_connected_is_eisconn() {
        let (mut s, _q, p) = setup_event();
        assert_eq!(s.port_destroy(p), Err(CellError::EISCONN));
    }

    #[test]
    fn evreg_queue_destroy_with_events_default_mode_is_ebusy() {
        let (mut s, q, p) = setup_event();
        s.port_send(p, 0, 0, 0).unwrap();
        assert_eq!(s.queue_destroy(q, 0), Err(CellError::EBUSY));
        // FORCE mode succeeds.
        assert_eq!(s.queue_destroy(q, QUEUE_DESTROY_FORCE), Ok(()));
    }

    // -- Reserved kinds compile and round-trip without crash ------

    #[test]
    fn reserved_kinds_allocate_destroy_cleanly() {
        let mut s = Lv2SyncState::new();
        for k in [
            Lv2SyncKind::Mutex,
            Lv2SyncKind::Sema,
            Lv2SyncKind::Cond,
            Lv2SyncKind::LwCond,
            Lv2SyncKind::EventFlag,
            Lv2SyncKind::EventQueue,
            Lv2SyncKind::EventPort,
            Lv2SyncKind::RwLock,
        ] {
            let id = s.allocate(k);
            assert_eq!(id.kind(), k);
            s.destroy(id.raw(), k).unwrap();
        }
        assert!(s.is_empty());
    }
}
