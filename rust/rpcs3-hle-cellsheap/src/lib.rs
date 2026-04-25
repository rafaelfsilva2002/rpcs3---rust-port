//! Rust port of `rpcs3/Emu/Cell/Modules/cellSheap.cpp`.
//!
//! 18 PRX entry points under the module name `cellSheap` split in two
//! logical surfaces:
//!
//!  - **cellSheap core** (5 entries): Initialize/Allocate/Free/QueryMax/
//!    QueryFree — a user-mode heap primitive.
//!  - **cellKeySheap** (13 entries): a key-addressed family of shared
//!    sync primitives (Buffer / Mutex / Barrier / Semaphore / Rwm /
//!    Queue), each paired with a `New` / `Delete` entry point.
//!
//! The C++ source (162 lines) returns `CELL_OK` from every entry body
//! via `UNIMPLEMENTED_FUNC`. The Rust port preserves that happy-path
//! semantics and layers FSM + key-uniqueness enforcement on top — so
//! callers that mis-sequence calls or double-register a key get a named
//! error instead of silent success.
//!
//! Module name byte-exact at cpp:4 `LOG_CHANNEL(cellSheap)` and cpp:140
//! `DECLARE(ppu_module_manager::cellSheap)("cellSheap", ...)`.
//!
//! Error codes are byte-exact with the `CellSheapError` enum at cpp:7-13:
//!
//! | name                       | value         |
//! |----------------------------|---------------|
//! | `CELL_SHEAP_ERROR_INVAL`   | `0x8041_0302` |
//! | `CELL_SHEAP_ERROR_BUSY`    | `0x8041_030A` |
//! | `CELL_SHEAP_ERROR_ALIGN`   | `0x8041_0310` |
//! | `CELL_SHEAP_ERROR_SHORTAGE`| `0x8041_0312` |

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

/// Byte-exact at cpp:4 / cpp:140.
pub const MODULE_NAME: &str = "cellSheap";

/// REG_FUNC order at cpp:142-161.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    // Core heap
    "cellSheapInitialize",
    "cellSheapAllocate",
    "cellSheapFree",
    "cellSheapQueryMax",
    "cellSheapQueryFree",
    // Key-sheap
    "cellKeySheapInitialize",
    "cellKeySheapBufferNew",
    "cellKeySheapBufferDelete",
    "cellKeySheapMutexNew",
    "cellKeySheapMutexDelete",
    "cellKeySheapBarrierNew",
    "cellKeySheapBarrierDelete",
    "cellKeySheapSemaphoreNew",
    "cellKeySheapSemaphoreDelete",
    "cellKeySheapRwmNew",
    "cellKeySheapRwmDelete",
    "cellKeySheapQueueNew",
    "cellKeySheapQueueDelete",
];

// --- Error codes (byte-exact cpp:7-13) ----------------------------------

pub const CELL_SHEAP_ERROR_INVAL: CellError = CellError(0x8041_0302);
pub const CELL_SHEAP_ERROR_BUSY: CellError = CellError(0x8041_030A);
pub const CELL_SHEAP_ERROR_ALIGN: CellError = CellError(0x8041_0310);
pub const CELL_SHEAP_ERROR_SHORTAGE: CellError = CellError(0x8041_0312);

// --- Key-sheap object kind ----------------------------------------------

/// Which primitive a key-addressed object refers to. Matches the
/// `New` / `Delete` pairs at cpp:68-137.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySheapKind {
    Buffer,
    Mutex,
    Barrier,
    Semaphore,
    Rwm,
    Queue,
}

/// Key-addressed object stored in the sheap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeySheapObject {
    pub key: u64,
    pub kind: KeySheapKind,
    pub size: u64,
    pub alignment: u64,
}

// --- Allocation bookkeeping ---------------------------------------------

/// Live allocation returned by `cellSheapAllocate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SheapAllocation {
    pub addr: u32,
    pub size: u64,
    pub alignment: u64,
}

/// Power-of-two check. Reproduces the cpp-style `(n & (n - 1)) == 0`
/// test the firmware uses to validate alignment. Zero is rejected (the
/// real sheap treats `0` alignment as "use default").
#[must_use]
pub const fn is_power_of_two(n: u64) -> bool {
    n != 0 && (n & (n - 1)) == 0
}

// --- FSM -----------------------------------------------------------------

/// Core-sheap lifecycle. `Uninitialized → Initialized` — there is no
/// documented `Finalize` entry (cpp has only `cellSheapInitialize`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheapState {
    Uninitialized,
    Initialized,
}

// --- Manager ------------------------------------------------------------

/// HLE state for both the core heap and its key-addressed companion.
///
/// The real firmware partitions a caller-supplied block of memory; the
/// port keeps a bump-allocator mirror so callers can exercise
/// Allocate/Free/QueryMax/QueryFree without wiring up the guest address
/// space.
#[derive(Debug, Default)]
pub struct Sheap {
    state: Option<SheapState>,
    key_state: Option<SheapState>,
    capacity: u64,
    bump: u64,
    allocations: Vec<SheapAllocation>,
    key_objects: Vec<KeySheapObject>,
    // Counters — useful for smoke tests.
    initialize_calls: u32,
    allocate_calls: u32,
    free_calls: u32,
    query_max_calls: u32,
    query_free_calls: u32,
    key_initialize_calls: u32,
}

impl Sheap {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: None,
            key_state: None,
            capacity: 0,
            bump: 0,
            allocations: Vec::new(),
            key_objects: Vec::new(),
            initialize_calls: 0,
            allocate_calls: 0,
            free_calls: 0,
            query_max_calls: 0,
            query_free_calls: 0,
            key_initialize_calls: 0,
        }
    }

    #[must_use]
    pub fn state(&self) -> SheapState {
        self.state.unwrap_or(SheapState::Uninitialized)
    }

    #[must_use]
    pub fn key_state(&self) -> SheapState {
        self.key_state.unwrap_or(SheapState::Uninitialized)
    }

    #[must_use]
    pub fn capacity(&self) -> u64 {
        self.capacity
    }

    #[must_use]
    pub fn used(&self) -> u64 {
        self.allocations.iter().map(|a| a.size).sum()
    }

    #[must_use]
    pub fn free_bytes(&self) -> u64 {
        self.capacity.saturating_sub(self.used())
    }

    #[must_use]
    pub fn live_allocations(&self) -> usize {
        self.allocations.len()
    }

    #[must_use]
    pub fn key_object_count(&self) -> usize {
        self.key_objects.len()
    }

    #[must_use]
    pub fn initialize_calls(&self) -> u32 {
        self.initialize_calls
    }
    #[must_use]
    pub fn allocate_calls(&self) -> u32 {
        self.allocate_calls
    }
    #[must_use]
    pub fn free_calls(&self) -> u32 {
        self.free_calls
    }
    #[must_use]
    pub fn query_max_calls(&self) -> u32 {
        self.query_max_calls
    }
    #[must_use]
    pub fn query_free_calls(&self) -> u32 {
        self.query_free_calls
    }
    #[must_use]
    pub fn key_initialize_calls(&self) -> u32 {
        self.key_initialize_calls
    }

    fn require_initialized(&self) -> Result<(), CellError> {
        if self.state() == SheapState::Initialized {
            Ok(())
        } else {
            Err(CELL_SHEAP_ERROR_INVAL)
        }
    }

    fn require_key_initialized(&self) -> Result<(), CellError> {
        if self.key_state() == SheapState::Initialized {
            Ok(())
        } else {
            Err(CELL_SHEAP_ERROR_INVAL)
        }
    }

    // --- core sheap entry points --------------------------------------

    /// `cellSheapInitialize` (cpp:32-36). Sets the heap capacity
    /// (the real firmware backs this with a caller-supplied buffer);
    /// double-init yields `BUSY` so callers don't silently trample the
    /// existing arena.
    pub fn initialize(&mut self, capacity: u64) -> Result<(), CellError> {
        if self.state() == SheapState::Initialized {
            return Err(CELL_SHEAP_ERROR_BUSY);
        }
        if capacity == 0 {
            return Err(CELL_SHEAP_ERROR_INVAL);
        }
        self.capacity = capacity;
        self.bump = 0;
        self.state = Some(SheapState::Initialized);
        self.initialize_calls = self.initialize_calls.saturating_add(1);
        Ok(())
    }

    /// `cellSheapAllocate` (cpp:38-42). `size == 0` is invalid. The
    /// port enforces `is_power_of_two(alignment)` + rounds the bump
    /// cursor up; if the aligned slot exceeds capacity we return
    /// `SHORTAGE`.
    pub fn allocate(&mut self, size: u64, alignment: u64) -> Result<SheapAllocation, CellError> {
        self.require_initialized()?;
        if size == 0 {
            return Err(CELL_SHEAP_ERROR_INVAL);
        }
        let align = if alignment == 0 { 1 } else { alignment };
        if !is_power_of_two(align) {
            return Err(CELL_SHEAP_ERROR_ALIGN);
        }
        // Round up the bump cursor.
        let aligned = (self.bump + align - 1) & !(align - 1);
        let end = aligned
            .checked_add(size)
            .ok_or(CELL_SHEAP_ERROR_SHORTAGE)?;
        if end > self.capacity {
            return Err(CELL_SHEAP_ERROR_SHORTAGE);
        }
        // Synthesise a "pointer" so tests can round-trip free() by addr.
        let addr_u64 = 0x4000_0000u64 + aligned;
        let addr = addr_u64 as u32;
        self.bump = end;
        let alloc = SheapAllocation {
            addr,
            size,
            alignment: align,
        };
        self.allocations.push(alloc);
        self.allocate_calls = self.allocate_calls.saturating_add(1);
        Ok(alloc)
    }

    /// `cellSheapFree` (cpp:44-48). Rejects unknown addresses with
    /// `INVAL`. Order-independent — the real sheap is a heap not a
    /// stack.
    pub fn free(&mut self, addr: u32) -> Result<(), CellError> {
        self.require_initialized()?;
        let pos = self
            .allocations
            .iter()
            .position(|a| a.addr == addr)
            .ok_or(CELL_SHEAP_ERROR_INVAL)?;
        self.allocations.swap_remove(pos);
        self.free_calls = self.free_calls.saturating_add(1);
        Ok(())
    }

    /// `cellSheapQueryMax` (cpp:50-54). Returns the total capacity in
    /// bytes.
    pub fn query_max(&mut self) -> Result<u64, CellError> {
        self.require_initialized()?;
        self.query_max_calls = self.query_max_calls.saturating_add(1);
        Ok(self.capacity)
    }

    /// `cellSheapQueryFree` (cpp:56-60). Returns the current number of
    /// free bytes (capacity minus live allocation total).
    pub fn query_free(&mut self) -> Result<u64, CellError> {
        self.require_initialized()?;
        self.query_free_calls = self.query_free_calls.saturating_add(1);
        Ok(self.free_bytes())
    }

    // --- key-sheap entry points ---------------------------------------

    /// `cellKeySheapInitialize` (cpp:62-66).
    pub fn key_initialize(&mut self) -> Result<(), CellError> {
        if self.key_state() == SheapState::Initialized {
            return Err(CELL_SHEAP_ERROR_BUSY);
        }
        self.key_state = Some(SheapState::Initialized);
        self.key_initialize_calls = self.key_initialize_calls.saturating_add(1);
        Ok(())
    }

    /// Dispatch for every `cellKeySheap*New` entry (Buffer/Mutex/
    /// Barrier/Semaphore/Rwm/Queue). Rejects duplicate keys with
    /// `BUSY` — the firmware would surface the same via the kernel
    /// key registry.
    pub fn key_new(
        &mut self,
        kind: KeySheapKind,
        key: u64,
        size: u64,
        alignment: u64,
    ) -> Result<(), CellError> {
        self.require_key_initialized()?;
        let align = if alignment == 0 { 1 } else { alignment };
        if !is_power_of_two(align) {
            return Err(CELL_SHEAP_ERROR_ALIGN);
        }
        // `Buffer` / `Queue` need a size; the sync primitives don't —
        // but we still reject negative-shape inputs uniformly.
        if matches!(kind, KeySheapKind::Buffer | KeySheapKind::Queue) && size == 0 {
            return Err(CELL_SHEAP_ERROR_INVAL);
        }
        if self.key_objects.iter().any(|o| o.key == key) {
            return Err(CELL_SHEAP_ERROR_BUSY);
        }
        self.key_objects.push(KeySheapObject {
            key,
            kind,
            size,
            alignment: align,
        });
        Ok(())
    }

    /// Dispatch for every `cellKeySheap*Delete` entry. Rejects unknown
    /// keys or mismatched kinds with `INVAL`.
    pub fn key_delete(&mut self, kind: KeySheapKind, key: u64) -> Result<(), CellError> {
        self.require_key_initialized()?;
        let pos = self
            .key_objects
            .iter()
            .position(|o| o.key == key && o.kind == kind)
            .ok_or(CELL_SHEAP_ERROR_INVAL)?;
        self.key_objects.swap_remove(pos);
        Ok(())
    }

    // Convenience wrappers for each cpp entry point.

    pub fn key_buffer_new(&mut self, key: u64, size: u64, alignment: u64) -> Result<(), CellError> {
        self.key_new(KeySheapKind::Buffer, key, size, alignment)
    }
    pub fn key_buffer_delete(&mut self, key: u64) -> Result<(), CellError> {
        self.key_delete(KeySheapKind::Buffer, key)
    }
    pub fn key_mutex_new(&mut self, key: u64, alignment: u64) -> Result<(), CellError> {
        self.key_new(KeySheapKind::Mutex, key, 0, alignment)
    }
    pub fn key_mutex_delete(&mut self, key: u64) -> Result<(), CellError> {
        self.key_delete(KeySheapKind::Mutex, key)
    }
    pub fn key_barrier_new(&mut self, key: u64, alignment: u64) -> Result<(), CellError> {
        self.key_new(KeySheapKind::Barrier, key, 0, alignment)
    }
    pub fn key_barrier_delete(&mut self, key: u64) -> Result<(), CellError> {
        self.key_delete(KeySheapKind::Barrier, key)
    }
    pub fn key_semaphore_new(&mut self, key: u64, alignment: u64) -> Result<(), CellError> {
        self.key_new(KeySheapKind::Semaphore, key, 0, alignment)
    }
    pub fn key_semaphore_delete(&mut self, key: u64) -> Result<(), CellError> {
        self.key_delete(KeySheapKind::Semaphore, key)
    }
    pub fn key_rwm_new(&mut self, key: u64, alignment: u64) -> Result<(), CellError> {
        self.key_new(KeySheapKind::Rwm, key, 0, alignment)
    }
    pub fn key_rwm_delete(&mut self, key: u64) -> Result<(), CellError> {
        self.key_delete(KeySheapKind::Rwm, key)
    }
    pub fn key_queue_new(&mut self, key: u64, size: u64, alignment: u64) -> Result<(), CellError> {
        self.key_new(KeySheapKind::Queue, key, size, alignment)
    }
    pub fn key_queue_delete(&mut self, key: u64) -> Result<(), CellError> {
        self.key_delete(KeySheapKind::Queue, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_name_byte_exact() {
        assert_eq!(MODULE_NAME, "cellSheap");
    }

    #[test]
    fn registered_entry_points_exact_order() {
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 18);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellSheapInitialize");
        assert_eq!(REGISTERED_ENTRY_POINTS[4], "cellSheapQueryFree");
        assert_eq!(REGISTERED_ENTRY_POINTS[5], "cellKeySheapInitialize");
        assert_eq!(REGISTERED_ENTRY_POINTS[17], "cellKeySheapQueueDelete");
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(CELL_SHEAP_ERROR_INVAL.0, 0x8041_0302);
        assert_eq!(CELL_SHEAP_ERROR_BUSY.0, 0x8041_030A);
        assert_eq!(CELL_SHEAP_ERROR_ALIGN.0, 0x8041_0310);
        assert_eq!(CELL_SHEAP_ERROR_SHORTAGE.0, 0x8041_0312);
    }

    #[test]
    fn is_power_of_two_cases() {
        assert!(!is_power_of_two(0));
        assert!(is_power_of_two(1));
        assert!(is_power_of_two(2));
        assert!(is_power_of_two(4));
        assert!(is_power_of_two(16));
        assert!(is_power_of_two(64));
        assert!(is_power_of_two(1024));
        assert!(!is_power_of_two(3));
        assert!(!is_power_of_two(5));
        assert!(!is_power_of_two(100));
    }

    #[test]
    fn initialize_happy_path() {
        let mut s = Sheap::new();
        s.initialize(0x10000).unwrap();
        assert_eq!(s.state(), SheapState::Initialized);
        assert_eq!(s.capacity(), 0x10000);
        assert_eq!(s.free_bytes(), 0x10000);
    }

    #[test]
    fn initialize_zero_capacity_is_inval() {
        let mut s = Sheap::new();
        assert_eq!(s.initialize(0), Err(CELL_SHEAP_ERROR_INVAL));
    }

    #[test]
    fn double_initialize_is_busy() {
        let mut s = Sheap::new();
        s.initialize(0x1000).unwrap();
        assert_eq!(s.initialize(0x1000), Err(CELL_SHEAP_ERROR_BUSY));
    }

    #[test]
    fn allocate_without_init_is_inval() {
        let mut s = Sheap::new();
        assert_eq!(s.allocate(16, 16), Err(CELL_SHEAP_ERROR_INVAL));
    }

    #[test]
    fn allocate_zero_size_is_inval() {
        let mut s = Sheap::new();
        s.initialize(0x1000).unwrap();
        assert_eq!(s.allocate(0, 16), Err(CELL_SHEAP_ERROR_INVAL));
    }

    #[test]
    fn allocate_non_power_of_two_alignment_is_align() {
        let mut s = Sheap::new();
        s.initialize(0x1000).unwrap();
        assert_eq!(s.allocate(16, 3), Err(CELL_SHEAP_ERROR_ALIGN));
    }

    #[test]
    fn allocate_past_capacity_is_shortage() {
        let mut s = Sheap::new();
        s.initialize(0x100).unwrap();
        assert_eq!(s.allocate(0x200, 16), Err(CELL_SHEAP_ERROR_SHORTAGE));
    }

    #[test]
    fn allocate_and_free_roundtrip() {
        let mut s = Sheap::new();
        s.initialize(0x1000).unwrap();
        let a = s.allocate(0x80, 16).unwrap();
        assert_eq!(s.live_allocations(), 1);
        assert_eq!(s.free_bytes(), 0x1000 - 0x80);
        s.free(a.addr).unwrap();
        assert_eq!(s.live_allocations(), 0);
    }

    #[test]
    fn free_unknown_addr_is_inval() {
        let mut s = Sheap::new();
        s.initialize(0x1000).unwrap();
        assert_eq!(s.free(0xDEAD_BEEF), Err(CELL_SHEAP_ERROR_INVAL));
    }

    #[test]
    fn allocate_honours_alignment() {
        let mut s = Sheap::new();
        s.initialize(0x1000).unwrap();
        // Consume 1 byte with alignment 1 so the bump cursor is at 1.
        let _ = s.allocate(1, 1).unwrap();
        // Next alloc with align=64 should round bump up to 0x40.
        let a = s.allocate(8, 64).unwrap();
        assert_eq!(u64::from(a.addr) % 64, 0);
    }

    #[test]
    fn query_max_and_free_match_bookkeeping() {
        let mut s = Sheap::new();
        s.initialize(0x800).unwrap();
        assert_eq!(s.query_max().unwrap(), 0x800);
        let _ = s.allocate(0x100, 16).unwrap();
        assert_eq!(s.query_free().unwrap(), 0x700);
    }

    #[test]
    fn key_initialize_happy_path() {
        let mut s = Sheap::new();
        s.key_initialize().unwrap();
        assert_eq!(s.key_state(), SheapState::Initialized);
    }

    #[test]
    fn key_double_init_is_busy() {
        let mut s = Sheap::new();
        s.key_initialize().unwrap();
        assert_eq!(s.key_initialize(), Err(CELL_SHEAP_ERROR_BUSY));
    }

    #[test]
    fn key_ops_without_init_are_inval() {
        let mut s = Sheap::new();
        assert_eq!(
            s.key_mutex_new(1, 16),
            Err(CELL_SHEAP_ERROR_INVAL)
        );
    }

    #[test]
    fn key_buffer_zero_size_is_inval() {
        let mut s = Sheap::new();
        s.key_initialize().unwrap();
        assert_eq!(s.key_buffer_new(7, 0, 16), Err(CELL_SHEAP_ERROR_INVAL));
    }

    #[test]
    fn key_new_bad_alignment_is_align() {
        let mut s = Sheap::new();
        s.key_initialize().unwrap();
        assert_eq!(
            s.key_mutex_new(1, 7),
            Err(CELL_SHEAP_ERROR_ALIGN)
        );
    }

    #[test]
    fn key_duplicate_is_busy() {
        let mut s = Sheap::new();
        s.key_initialize().unwrap();
        s.key_mutex_new(42, 16).unwrap();
        assert_eq!(s.key_mutex_new(42, 16), Err(CELL_SHEAP_ERROR_BUSY));
    }

    #[test]
    fn key_delete_unknown_is_inval() {
        let mut s = Sheap::new();
        s.key_initialize().unwrap();
        assert_eq!(
            s.key_mutex_delete(99),
            Err(CELL_SHEAP_ERROR_INVAL)
        );
    }

    #[test]
    fn key_delete_kind_mismatch_is_inval() {
        let mut s = Sheap::new();
        s.key_initialize().unwrap();
        s.key_mutex_new(7, 16).unwrap();
        // Same key but wrong kind.
        assert_eq!(
            s.key_barrier_delete(7),
            Err(CELL_SHEAP_ERROR_INVAL)
        );
    }

    #[test]
    fn key_each_kind_roundtrips() {
        let mut s = Sheap::new();
        s.key_initialize().unwrap();
        s.key_buffer_new(10, 0x100, 16).unwrap();
        s.key_mutex_new(11, 16).unwrap();
        s.key_barrier_new(12, 16).unwrap();
        s.key_semaphore_new(13, 16).unwrap();
        s.key_rwm_new(14, 16).unwrap();
        s.key_queue_new(15, 0x40, 16).unwrap();
        assert_eq!(s.key_object_count(), 6);
        s.key_buffer_delete(10).unwrap();
        s.key_mutex_delete(11).unwrap();
        s.key_barrier_delete(12).unwrap();
        s.key_semaphore_delete(13).unwrap();
        s.key_rwm_delete(14).unwrap();
        s.key_queue_delete(15).unwrap();
        assert_eq!(s.key_object_count(), 0);
    }

    #[test]
    fn full_sheap_lifecycle_smoke() {
        let mut s = Sheap::new();

        // 1. Core heap init + a handful of allocations of varying
        //    alignment.
        s.initialize(0x10000).unwrap();
        let a = s.allocate(0x40, 16).unwrap();
        let b = s.allocate(0x80, 64).unwrap();
        let c = s.allocate(0x100, 128).unwrap();
        assert_eq!(s.live_allocations(), 3);
        assert!(u64::from(b.addr) % 64 == 0);
        assert!(u64::from(c.addr) % 128 == 0);

        // 2. Free in non-stack order — the heap tolerates that.
        s.free(b.addr).unwrap();
        assert_eq!(s.live_allocations(), 2);

        // 3. Key-sheap primitives live independently.
        s.key_initialize().unwrap();
        s.key_mutex_new(0xA, 16).unwrap();
        s.key_barrier_new(0xB, 16).unwrap();
        s.key_buffer_new(0xC, 0x80, 16).unwrap();
        assert_eq!(s.key_object_count(), 3);

        // 4. Query the residual capacity.
        let max = s.query_max().unwrap();
        let free = s.query_free().unwrap();
        assert_eq!(max, 0x10000);
        assert_eq!(max - free, a.size + c.size);

        // 5. Clean up keys + remaining allocations.
        s.key_mutex_delete(0xA).unwrap();
        s.key_barrier_delete(0xB).unwrap();
        s.key_buffer_delete(0xC).unwrap();
        s.free(a.addr).unwrap();
        s.free(c.addr).unwrap();
        assert_eq!(s.live_allocations(), 0);
        assert_eq!(s.key_object_count(), 0);

        // 6. Counter trace.
        assert_eq!(s.initialize_calls(), 1);
        assert_eq!(s.allocate_calls(), 3);
        assert_eq!(s.free_calls(), 3);
        assert_eq!(s.query_max_calls(), 1);
        assert_eq!(s.query_free_calls(), 1);
        assert_eq!(s.key_initialize_calls(), 1);
    }
}
