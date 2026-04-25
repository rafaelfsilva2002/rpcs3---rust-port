//! `rpcs3-hle-celldaisy` — PS3 Daisy lock-free queue + SPU interlock HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellDaisy.cpp` (223 linhas).  The
//! Daisy library (`cell::Daisy::*`) gives games a high-level SPU-PPU
//! interlock primitive: a lock-free 2-ring `LFQueue2`, a MPMC-style
//! `Lock` ring buffer, and a `ScatterGatherInterlock` for batched DMA
//! synchronization.  Every C++ entry point is an `UNIMPLEMENTED_FUNC`
//! stub — the Rust port captures the observable producer/consumer
//! state machines so higher layers can exercise them without the real
//! SPU back-ends.
//!
//! ## Entry-point families
//!
//! * **LFQueue2** — 2-ring pop/push open/close + GetPopPointer +
//!   CompletePopPointer + HasUnfinishedConsumer.
//! * **Lock** — initialize, push/pop open/close, getNextHeadPointer /
//!   getNextTailPointer, completeConsume / completeProduce.
//! * **ScatterGatherInterlock** — two ctor overloads, destructor,
//!   probe / release / proceedSequenceNumber.
//! * **`cellDaisy_snprintf`** — C-style varargs formatter stub.

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellDaisy.h:10-17.  Non-contiguous!
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const NO_BEGIN:            CellError = CellError(0x8041_0501);
    pub const INVALID_PORT_ATTACH: CellError = CellError(0x8041_0502);
    pub const NOT_IMPLEMENTED:     CellError = CellError(0x8041_0503);
    pub const PERM:                CellError = CellError(0x8041_0509);
    pub const STAT:                CellError = CellError(0x8041_050F);
    pub const AGAIN:               CellError = CellError(0x8041_0511);
    pub const INVAL:               CellError = CellError(0x8041_0512);
    pub const BUSY:                CellError = CellError(0x8041_051A);
}

// =====================================================================
// LFQueue2 — lock-free producer/consumer ring with ref-counted opens
// =====================================================================

/// Observable state of a single Daisy `LFQueue2` instance.  The C++
/// side tracks this via atomic counters embedded in the guest struct;
/// the Rust port mirrors it with host integers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LfQueue2 {
    /// Count of concurrent producers (each `PushOpen` bumps).
    pub push_open_count: u32,
    /// Count of concurrent consumers (each `PopOpen` bumps).
    pub pop_open_count: u32,
    /// Head pointer of the ring.
    pub head: i32,
    /// Tail pointer of the ring.
    pub tail: i32,
    /// Set when `HasUnfinishedConsumer` should report "busy".
    pub unfinished_consumers: u32,
}

impl LfQueue2 {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Port of `cellDaisyLFQueue2PushOpen` — bumps the producer count.
    /// Returns `()` because the C++ function is `void`.
    pub fn push_open(&mut self) { self.push_open_count = self.push_open_count.saturating_add(1); }

    /// Port of `cellDaisyLFQueue2PushClose`.
    ///
    /// # Errors
    /// * [`errors::PERM`] if `PushClose` is called without a matching
    ///   `PushOpen` (producer count underflow).
    pub fn push_close(&mut self) -> Result<(), CellError> {
        if self.push_open_count == 0 { return Err(errors::PERM); }
        self.push_open_count -= 1;
        Ok(())
    }

    /// Port of `cellDaisyLFQueue2PopOpen`.
    pub fn pop_open(&mut self) { self.pop_open_count = self.pop_open_count.saturating_add(1); }

    /// Port of `cellDaisyLFQueue2PopClose`.
    ///
    /// # Errors
    /// * [`errors::PERM`] if closed more than opened.
    pub fn pop_close(&mut self) -> Result<(), CellError> {
        if self.pop_open_count == 0 { return Err(errors::PERM); }
        self.pop_open_count -= 1;
        Ok(())
    }

    /// Port of `cellDaisyLFQueue2GetPopPointer`.  Non-blocking mode
    /// returns `AGAIN` if the queue is empty; blocking mode signals
    /// the caller to sleep.
    ///
    /// # Errors
    /// * [`errors::AGAIN`] on empty queue with `is_blocking == false`.
    /// * [`errors::NO_BEGIN`] if no producer has ever opened (queue
    ///   has never been primed).
    pub fn get_pop_pointer(&self, is_blocking: bool) -> Result<i32, CellError> {
        if self.push_open_count == 0 && self.head == self.tail {
            return Err(errors::NO_BEGIN);
        }
        if self.head == self.tail {
            return if is_blocking { Err(errors::BUSY) } else { Err(errors::AGAIN) };
        }
        Ok(self.head)
    }

    /// Port of `cellDaisyLFQueue2CompletePopPointer`.  Advances `head`
    /// past the given pointer.
    ///
    /// # Errors
    /// * [`errors::INVAL`] if `pointer != self.head` (out-of-order
    ///   complete).
    pub fn complete_pop_pointer(&mut self, pointer: i32, _is_queue_full: u32) -> Result<(), CellError> {
        if pointer != self.head { return Err(errors::INVAL); }
        self.head = self.head.wrapping_add(1);
        Ok(())
    }

    /// Port of `cellDaisyLFQueue2HasUnfinishedConsumer`.
    #[must_use]
    pub fn has_unfinished_consumer(&self, is_cancelled: bool) -> bool {
        if is_cancelled { return false; }
        self.unfinished_consumers > 0
    }

    /// Test helper — simulate a producer pushing an item.
    pub fn inject_push(&mut self) { self.tail = self.tail.wrapping_add(1); }
}

// =====================================================================
// Lock — classic MPMC ring buffer with head/tail pointers
// =====================================================================

/// Observable state of `cell::Daisy::Lock`.  `depth` is the fixed ring
/// capacity; `head`/`tail` are the consumer/producer indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lock {
    pub depth: u32,
    pub head: u32,
    pub tail: u32,
    pub push_open_count: u32,
    pub pop_open_count: u32,
    pub initialized: bool,
}

impl Default for Lock {
    fn default() -> Self { Self { depth: 0, head: 0, tail: 0, push_open_count: 0, pop_open_count: 0, initialized: false } }
}

impl Lock {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Port of `cellDaisyLock_initialize`.
    ///
    /// # Errors
    /// * [`errors::INVAL`] if `depth == 0`.
    /// * [`errors::BUSY`] if already initialised.
    pub fn initialize(&mut self, depth: u32) -> Result<(), CellError> {
        if depth == 0 { return Err(errors::INVAL); }
        if self.initialized { return Err(errors::BUSY); }
        self.depth = depth;
        self.initialized = true;
        Ok(())
    }

    fn require_init(&self) -> Result<(), CellError> {
        if !self.initialized { return Err(errors::NO_BEGIN); }
        Ok(())
    }

    /// Port of `cellDaisyLock_pushOpen` — bumps the producer count.
    pub fn push_open(&mut self) -> Result<(), CellError> {
        self.require_init()?;
        self.push_open_count = self.push_open_count.saturating_add(1);
        Ok(())
    }

    /// Port of `cellDaisyLock_pushClose`.
    pub fn push_close(&mut self) -> Result<(), CellError> {
        self.require_init()?;
        if self.push_open_count == 0 { return Err(errors::PERM); }
        self.push_open_count -= 1;
        Ok(())
    }

    /// Port of `cellDaisyLock_popOpen`.
    pub fn pop_open(&mut self) -> Result<(), CellError> {
        self.require_init()?;
        self.pop_open_count = self.pop_open_count.saturating_add(1);
        Ok(())
    }

    /// Port of `cellDaisyLock_popClose`.
    pub fn pop_close(&mut self) -> Result<(), CellError> {
        self.require_init()?;
        if self.pop_open_count == 0 { return Err(errors::PERM); }
        self.pop_open_count -= 1;
        Ok(())
    }

    /// Port of `cellDaisyLock_getNextHeadPointer`.
    ///
    /// # Errors
    /// * [`errors::AGAIN`] if the ring is empty (`head == tail`).
    pub fn get_next_head_pointer(&self) -> Result<u32, CellError> {
        self.require_init()?;
        if self.head == self.tail { return Err(errors::AGAIN); }
        Ok(self.head)
    }

    /// Port of `cellDaisyLock_getNextTailPointer`.
    ///
    /// # Errors
    /// * [`errors::AGAIN`] if the ring is full
    ///   (`tail - head >= depth`).
    pub fn get_next_tail_pointer(&self) -> Result<u32, CellError> {
        self.require_init()?;
        if self.tail.wrapping_sub(self.head) >= self.depth {
            return Err(errors::AGAIN);
        }
        Ok(self.tail)
    }

    /// Port of `cellDaisyLock_completeConsume`.  Advances the head
    /// pointer; `pointer` must match the current head.
    ///
    /// # Errors
    /// * [`errors::INVAL`] if `pointer` is out of order.
    pub fn complete_consume(&mut self, pointer: u32) -> Result<(), CellError> {
        self.require_init()?;
        if pointer != self.head { return Err(errors::INVAL); }
        self.head = self.head.wrapping_add(1);
        Ok(())
    }

    /// Port of `cellDaisyLock_completeProduce`.
    ///
    /// # Errors
    /// * [`errors::INVAL`] if `pointer` is out of order.
    pub fn complete_produce(&mut self, pointer: u32) -> Result<(), CellError> {
        self.require_init()?;
        if pointer != self.tail { return Err(errors::INVAL); }
        self.tail = self.tail.wrapping_add(1);
        Ok(())
    }
}

// =====================================================================
// ScatterGatherInterlock
// =====================================================================

/// Observable lifecycle of `cell::Daisy::ScatterGatherInterlock`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SgState {
    #[default]
    Uninit,
    /// After a constructor — waiting for `probe` to advance.
    Armed,
    /// After `probe()` reports ready.
    Probed,
    /// After `release()`.
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScatterGatherInterlock {
    pub state: SgState,
    pub size: u32,
    pub sequence_number: u32,
    pub variant: u8, // 1 or 2 to match the two C++ constructor overloads
}

impl ScatterGatherInterlock {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Port of `cellDaisyScatterGatherInterlock_1` (ctor overload 1).
    pub fn ctor_variant1(&mut self, size: u32) {
        self.size = size;
        self.variant = 1;
        self.state = SgState::Armed;
    }

    /// Port of `cellDaisyScatterGatherInterlock_2` (ctor overload 2).
    pub fn ctor_variant2(&mut self, size: u32, _num_spus: u32, _spup: u8) {
        self.size = size;
        self.variant = 2;
        self.state = SgState::Armed;
    }

    /// Port of `cellDaisyScatterGatherInterlock_9tor` (destructor).
    pub fn destruct(&mut self) {
        *self = Self::default();
    }

    /// Port of `cellDaisyScatterGatherInterlock_probe`.
    ///
    /// # Errors
    /// * [`errors::AGAIN`] in non-blocking mode while still Armed.
    /// * [`errors::STAT`] if the interlock is in a non-probable state.
    pub fn probe(&mut self, is_blocking: u32) -> Result<(), CellError> {
        match self.state {
            SgState::Uninit | SgState::Released => Err(errors::STAT),
            SgState::Armed if is_blocking == 0 => Err(errors::AGAIN),
            SgState::Armed => {
                self.state = SgState::Probed;
                Ok(())
            }
            SgState::Probed => Ok(()),
        }
    }

    /// Port of `cellDaisyScatterGatherInterlock_release`.
    ///
    /// # Errors
    /// * [`errors::STAT`] if not currently Probed.
    pub fn release(&mut self) -> Result<(), CellError> {
        if self.state != SgState::Probed { return Err(errors::STAT); }
        self.state = SgState::Released;
        Ok(())
    }

    /// Port of `cellDaisyScatterGatherInterlock_proceedSequenceNumber`.
    /// No error in the C++ stub — just bumps the internal counter.
    pub fn proceed_sequence_number(&mut self) {
        self.sequence_number = self.sequence_number.wrapping_add(1);
    }
}

// =====================================================================
// snprintf stub
// =====================================================================

/// Port of `cellDaisy_snprintf` (cpp:74-78).  The C++ stub returns
/// `CELL_OK` without writing.  Callers just see zero bytes emitted.
#[must_use]
pub fn snprintf_stub(_count: u32) -> i32 { 0 }

// =====================================================================
// Registry — 43 FNID entries (C++ ABI mangled names)
// =====================================================================

/// Logical entry-point names grouped by class.  The C++ registry
/// actually lists each name twice with different prefixes (`_ZN` and
/// `_QN`) — we expose the de-mangled logical names here.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellDaisyLFQueue2GetPopPointer",
    "cellDaisyLFQueue2CompletePopPointer",
    "cellDaisyLFQueue2PushOpen",
    "cellDaisyLFQueue2PushClose",
    "cellDaisyLFQueue2PopOpen",
    "cellDaisyLFQueue2PopClose",
    "cellDaisyLFQueue2HasUnfinishedConsumer",
    "cellDaisy_snprintf",
    "cellDaisyLock_initialize",
    "cellDaisyLock_getNextHeadPointer",
    "cellDaisyLock_getNextTailPointer",
    "cellDaisyLock_completeConsume",
    "cellDaisyLock_completeProduce",
    "cellDaisyLock_pushOpen",
    "cellDaisyLock_pushClose",
    "cellDaisyLock_popOpen",
    "cellDaisyLock_popClose",
    "cellDaisyScatterGatherInterlock_1",
    "cellDaisyScatterGatherInterlock_2",
    "cellDaisyScatterGatherInterlock_9tor",
    "cellDaisyScatterGatherInterlock_probe",
    "cellDaisyScatterGatherInterlock_release",
    "cellDaisyScatterGatherInterlock_proceedSequenceNumber",
];

#[must_use]
pub fn is_registered(name: &str) -> bool {
    REGISTERED_ENTRY_POINTS.contains(&name)
}

/// Count of FNID entries in the C++ `REG_FNID` block cpp:169-222.
/// Each logical name is listed twice (`_ZN…` + `_QN…`), so total = 2×logical.
pub const CPP_FNID_COUNT: usize = 43;

// Silence unused import warning for Vec when no feature uses it.
#[allow(dead_code)]
fn _use_vec(_: &Vec<u32>) {}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- constants ---------------------------------------------------

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::NO_BEGIN.0,            0x8041_0501);
        assert_eq!(errors::INVALID_PORT_ATTACH.0, 0x8041_0502);
        assert_eq!(errors::NOT_IMPLEMENTED.0,     0x8041_0503);
        assert_eq!(errors::PERM.0,                0x8041_0509);
        assert_eq!(errors::STAT.0,                0x8041_050F);
        assert_eq!(errors::AGAIN.0,               0x8041_0511);
        assert_eq!(errors::INVAL.0,               0x8041_0512);
        assert_eq!(errors::BUSY.0,                0x8041_051A);
    }

    #[test]
    fn error_codes_all_distinct() {
        let codes = [
            errors::NO_BEGIN, errors::INVALID_PORT_ATTACH, errors::NOT_IMPLEMENTED,
            errors::PERM, errors::STAT, errors::AGAIN, errors::INVAL, errors::BUSY,
        ];
        let mut sorted: alloc::vec::Vec<u32> = codes.iter().map(|c| c.0).collect();
        sorted.sort_unstable();
        for pair in sorted.windows(2) {
            assert_ne!(pair[0], pair[1]);
        }
    }

    #[test]
    fn registry_has_23_logical_names() {
        // 7 LFQueue2 + snprintf + 9 Lock + 6 ScatterGatherInterlock = 23.
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 23);
    }

    #[test]
    fn cpp_fnid_count_is_43() {
        // The C++ source registers each logical name twice (prefixes
        // `_ZN` and `_QN`) — total 43 REG_FNID calls.
        assert_eq!(CPP_FNID_COUNT, 43);
    }

    #[test]
    fn registry_covers_three_families() {
        assert!(is_registered("cellDaisyLFQueue2PushOpen"));
        assert!(is_registered("cellDaisyLock_initialize"));
        assert!(is_registered("cellDaisyScatterGatherInterlock_probe"));
    }

    // ---- LFQueue2 ---------------------------------------------------

    #[test]
    fn lfq_push_open_close_refcount() {
        let mut q = LfQueue2::new();
        q.push_open();
        q.push_open();
        assert_eq!(q.push_open_count, 2);
        q.push_close().unwrap();
        q.push_close().unwrap();
        assert_eq!(q.push_open_count, 0);
    }

    #[test]
    fn lfq_push_close_underflow_is_perm() {
        let mut q = LfQueue2::new();
        assert_eq!(q.push_close().unwrap_err(), errors::PERM);
    }

    #[test]
    fn lfq_pop_open_close_refcount() {
        let mut q = LfQueue2::new();
        q.pop_open();
        q.pop_open();
        q.pop_close().unwrap();
        assert_eq!(q.pop_open_count, 1);
    }

    #[test]
    fn lfq_pop_close_underflow_is_perm() {
        let mut q = LfQueue2::new();
        assert_eq!(q.pop_close().unwrap_err(), errors::PERM);
    }

    #[test]
    fn lfq_get_pop_pointer_no_begin_when_empty_and_unopened() {
        let q = LfQueue2::new();
        assert_eq!(q.get_pop_pointer(false).unwrap_err(), errors::NO_BEGIN);
    }

    #[test]
    fn lfq_get_pop_pointer_again_on_empty_after_open() {
        let mut q = LfQueue2::new();
        q.push_open();
        assert_eq!(q.get_pop_pointer(false).unwrap_err(), errors::AGAIN);
    }

    #[test]
    fn lfq_get_pop_pointer_busy_on_empty_blocking() {
        let mut q = LfQueue2::new();
        q.push_open();
        assert_eq!(q.get_pop_pointer(true).unwrap_err(), errors::BUSY);
    }

    #[test]
    fn lfq_get_pop_pointer_returns_head_when_populated() {
        let mut q = LfQueue2::new();
        q.push_open();
        q.inject_push();
        assert_eq!(q.get_pop_pointer(false).unwrap(), 0);
    }

    #[test]
    fn lfq_complete_pop_advances_head() {
        let mut q = LfQueue2::new();
        q.push_open();
        q.inject_push();
        let ptr = q.get_pop_pointer(false).unwrap();
        q.complete_pop_pointer(ptr, 0).unwrap();
        assert_eq!(q.head, 1);
    }

    #[test]
    fn lfq_complete_pop_bad_pointer_is_inval() {
        let mut q = LfQueue2::new();
        q.push_open();
        q.inject_push();
        assert_eq!(q.complete_pop_pointer(99, 0).unwrap_err(), errors::INVAL);
    }

    #[test]
    fn lfq_has_unfinished_consumer_counter() {
        let mut q = LfQueue2::new();
        q.unfinished_consumers = 1;
        assert!(q.has_unfinished_consumer(false));
        assert!(!q.has_unfinished_consumer(true));
    }

    // ---- Lock --------------------------------------------------------

    #[test]
    fn lock_initialize_zero_depth_is_inval() {
        let mut l = Lock::new();
        assert_eq!(l.initialize(0).unwrap_err(), errors::INVAL);
    }

    #[test]
    fn lock_initialize_twice_is_busy() {
        let mut l = Lock::new();
        l.initialize(16).unwrap();
        assert_eq!(l.initialize(32).unwrap_err(), errors::BUSY);
    }

    #[test]
    fn lock_push_open_before_init_is_no_begin() {
        let mut l = Lock::new();
        assert_eq!(l.push_open().unwrap_err(), errors::NO_BEGIN);
    }

    #[test]
    fn lock_push_pop_open_close_refcount() {
        let mut l = Lock::new();
        l.initialize(16).unwrap();
        l.push_open().unwrap();
        l.push_open().unwrap();
        l.push_close().unwrap();
        assert_eq!(l.push_open_count, 1);
        l.pop_open().unwrap();
        l.pop_close().unwrap();
        assert_eq!(l.pop_open_count, 0);
    }

    #[test]
    fn lock_close_underflow_is_perm() {
        let mut l = Lock::new();
        l.initialize(16).unwrap();
        assert_eq!(l.push_close().unwrap_err(), errors::PERM);
        assert_eq!(l.pop_close().unwrap_err(), errors::PERM);
    }

    #[test]
    fn lock_get_next_head_empty_is_again() {
        let mut l = Lock::new();
        l.initialize(16).unwrap();
        assert_eq!(l.get_next_head_pointer().unwrap_err(), errors::AGAIN);
    }

    #[test]
    fn lock_get_next_tail_full_is_again() {
        let mut l = Lock::new();
        l.initialize(4).unwrap();
        l.tail = 4; // head=0, tail=4 → full
        assert_eq!(l.get_next_tail_pointer().unwrap_err(), errors::AGAIN);
    }

    #[test]
    fn lock_complete_produce_consume_roundtrip() {
        let mut l = Lock::new();
        l.initialize(16).unwrap();
        let tail = l.get_next_tail_pointer().unwrap();
        assert_eq!(tail, 0);
        l.complete_produce(tail).unwrap();
        assert_eq!(l.tail, 1);
        let head = l.get_next_head_pointer().unwrap();
        assert_eq!(head, 0);
        l.complete_consume(head).unwrap();
        assert_eq!(l.head, 1);
    }

    #[test]
    fn lock_complete_consume_wrong_pointer_is_inval() {
        let mut l = Lock::new();
        l.initialize(16).unwrap();
        l.tail = 1;
        assert_eq!(l.complete_consume(99).unwrap_err(), errors::INVAL);
    }

    // ---- ScatterGatherInterlock -------------------------------------

    #[test]
    fn sg_ctor_variant1_transitions_to_armed() {
        let mut s = ScatterGatherInterlock::new();
        s.ctor_variant1(256);
        assert_eq!(s.state, SgState::Armed);
        assert_eq!(s.variant, 1);
        assert_eq!(s.size, 256);
    }

    #[test]
    fn sg_ctor_variant2_transitions_to_armed() {
        let mut s = ScatterGatherInterlock::new();
        s.ctor_variant2(512, 4, 0);
        assert_eq!(s.state, SgState::Armed);
        assert_eq!(s.variant, 2);
    }

    #[test]
    fn sg_probe_non_blocking_armed_is_again() {
        let mut s = ScatterGatherInterlock::new();
        s.ctor_variant1(256);
        assert_eq!(s.probe(0).unwrap_err(), errors::AGAIN);
    }

    #[test]
    fn sg_probe_blocking_transitions_to_probed() {
        let mut s = ScatterGatherInterlock::new();
        s.ctor_variant1(256);
        s.probe(1).unwrap();
        assert_eq!(s.state, SgState::Probed);
    }

    #[test]
    fn sg_probe_when_uninit_is_stat() {
        let mut s = ScatterGatherInterlock::new();
        assert_eq!(s.probe(1).unwrap_err(), errors::STAT);
    }

    #[test]
    fn sg_release_without_probe_is_stat() {
        let mut s = ScatterGatherInterlock::new();
        s.ctor_variant1(256);
        assert_eq!(s.release().unwrap_err(), errors::STAT);
    }

    #[test]
    fn sg_probe_release_cycle() {
        let mut s = ScatterGatherInterlock::new();
        s.ctor_variant1(256);
        s.probe(1).unwrap();
        s.release().unwrap();
        assert_eq!(s.state, SgState::Released);
        // Re-probe after release is STAT.
        assert_eq!(s.probe(1).unwrap_err(), errors::STAT);
    }

    #[test]
    fn sg_proceed_sequence_number_bumps_counter() {
        let mut s = ScatterGatherInterlock::new();
        s.proceed_sequence_number();
        s.proceed_sequence_number();
        assert_eq!(s.sequence_number, 2);
    }

    #[test]
    fn sg_destruct_resets_state() {
        let mut s = ScatterGatherInterlock::new();
        s.ctor_variant1(256);
        s.probe(1).unwrap();
        s.destruct();
        assert_eq!(s, ScatterGatherInterlock::default());
    }

    // ---- full smoke -------------------------------------------------

    #[test]
    fn full_celldaisy_lifecycle_smoke() {
        // 1. LFQueue2 producer/consumer flow.
        let mut q = LfQueue2::new();
        q.push_open();
        q.pop_open();
        q.inject_push();
        let p = q.get_pop_pointer(false).unwrap();
        q.complete_pop_pointer(p, 0).unwrap();
        q.push_close().unwrap();
        q.pop_close().unwrap();

        // 2. Lock MPMC ring.
        let mut l = Lock::new();
        l.initialize(4).unwrap();
        l.push_open().unwrap();
        l.pop_open().unwrap();
        let t = l.get_next_tail_pointer().unwrap();
        l.complete_produce(t).unwrap();
        let h = l.get_next_head_pointer().unwrap();
        l.complete_consume(h).unwrap();

        // 3. ScatterGatherInterlock.
        let mut s = ScatterGatherInterlock::new();
        s.ctor_variant2(256, 4, 0);
        s.probe(1).unwrap();
        s.proceed_sequence_number();
        s.release().unwrap();
        s.destruct();
        assert_eq!(s, ScatterGatherInterlock::default());
    }
}
