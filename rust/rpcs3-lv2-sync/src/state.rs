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
    // Variants below are placeholders the future slices flesh out.
    // They are populated by the registry once the corresponding R10.x
    // slice lands; until then they're unreachable in production code
    // because no `allocate(Lv2SyncKind::X)` callers exist yet.
    Mutex,
    Sema,
    Cond,
    LwCond,
    EventFlag,
    EventQueue,
    EventPort,
    RwLock,
}

impl Entry {
    fn kind(&self) -> Lv2SyncKind {
        match self {
            Entry::LwMutex(_) => Lv2SyncKind::LwMutex,
            Entry::Mutex => Lv2SyncKind::Mutex,
            Entry::Sema => Lv2SyncKind::Sema,
            Entry::Cond => Lv2SyncKind::Cond,
            Entry::LwCond => Lv2SyncKind::LwCond,
            Entry::EventFlag => Lv2SyncKind::EventFlag,
            Entry::EventQueue => Lv2SyncKind::EventQueue,
            Entry::EventPort => Lv2SyncKind::EventPort,
            Entry::RwLock => Lv2SyncKind::RwLock,
        }
    }

    fn new(kind: Lv2SyncKind) -> Self {
        match kind {
            Lv2SyncKind::LwMutex => Entry::LwMutex(LwMutex::default()),
            Lv2SyncKind::Mutex => Entry::Mutex,
            Lv2SyncKind::Sema => Entry::Sema,
            Lv2SyncKind::Cond => Entry::Cond,
            Lv2SyncKind::LwCond => Entry::LwCond,
            Lv2SyncKind::EventFlag => Entry::EventFlag,
            Lv2SyncKind::EventQueue => Entry::EventQueue,
            Lv2SyncKind::EventPort => Entry::EventPort,
            Lv2SyncKind::RwLock => Entry::RwLock,
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
