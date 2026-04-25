//! `rpcs3-hle-sys-heap` — PS3 user-mode heap + spinlock primitives.
//!
//! Ports:
//! * `rpcs3/Emu/Cell/Modules/sys_heap.cpp` — `_sys_heap_create_heap`,
//!   `_sys_heap_delete_heap`, `_sys_heap_malloc`, `_sys_heap_memalign`,
//!   `_sys_heap_free` (+ 4 stub wrappers).
//! * `rpcs3/Emu/Cell/Modules/sys_spinlock.cpp` — `sys_spinlock_initialize`,
//!   `sys_spinlock_lock`, `sys_spinlock_trylock`, `sys_spinlock_unlock`.
//!
//! Both are registered under `sysPrxForUser`.  The firmware delegates
//! heap operations to `vm::alloc`/`vm::dealloc`; the spinlock primitives
//! are genuine busy-wait loops using the sentinel value `0xabadcafe`.

extern crate alloc;

use alloc::string::String;
use rpcs3_emu_types::CellError;

// =====================================================================
// sys_spinlock.cpp — byte-exact constants
// =====================================================================

/// Sentinel value the PS3 firmware writes into a spinlock while it's
/// held — see `sys_spinlock_lock` in sys_spinlock.cpp:21
/// (`lock->exchange(0xabadcafe)`).
pub const SPINLOCK_HELD_SENTINEL: u32 = 0xABAD_CAFE;

/// Released value — any non-sentinel value technically works, but the
/// firmware always writes 0 in `sys_spinlock_unlock` / `initialize`.
pub const SPINLOCK_FREE_SENTINEL: u32 = 0;

/// `CELL_EBUSY` — returned by `sys_spinlock_trylock` on contention.
/// Value matches [cellOk.h] `SYS_EBUSY = 0x8001000A`.
pub const CELL_EBUSY: CellError = CellError(0x8001_000A);

// =====================================================================
// sys_heap.cpp — heap-id registry
// =====================================================================

/// `HeapInfo::id_base` from sys_heap.cpp:9.
pub const HEAP_ID_BASE: u32 = 1;

/// `HeapInfo::id_count` from sys_heap.cpp:11.
pub const HEAP_ID_COUNT: u32 = 1023;

/// Exclusive upper bound of the heap id range.
pub const HEAP_ID_LIMIT: u32 = HEAP_ID_BASE + HEAP_ID_COUNT;

/// Minimum alignment used by `_sys_heap_memalign` — the firmware
/// clamps to `max(align, 0x10000)` (64 KiB, matching `vm::main` page).
pub const MEMALIGN_MIN: u32 = 0x10000;

/// Named heap descriptor — mirror of `HeapInfo` in sys_heap.cpp:7-20.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeapInfo {
    pub id: u32,
    pub name: String,
    pub live_allocations: u32,
}

impl HeapInfo {
    #[must_use]
    pub fn new(id: u32, name: &str) -> Self {
        Self { id, name: String::from(name), live_allocations: 0 }
    }
}

/// `SysHeap` manager — owns a table of up to 1023 named heaps.  Each
/// allocation is modelled as a monotonic "address" so higher layers can
/// test the guest-visible identifier flow without touching `vm::alloc`.
#[derive(Debug, Default, Clone)]
pub struct SysHeap {
    heaps: alloc::vec::Vec<HeapInfo>,
    next_addr: u32,
}

impl SysHeap {
    #[must_use]
    pub fn new() -> Self {
        Self {
            heaps: alloc::vec::Vec::new(),
            // Start at a distinctive address so logs visually separate
            // heap allocations from library handles (0x1000_0000 etc).
            next_addr: 0x2000_0000,
        }
    }

    /// Port of `_sys_heap_create_heap`.  Returns the new heap id (or 0
    /// when the table is full, matching the `idm::make<HeapInfo>()`
    /// failure mode).
    ///
    /// # Errors
    /// Never errors — a full table returns id 0 (sentinel for failure)
    /// exactly as the C++ `idm::make` path does.
    pub fn create_heap(&mut self, name: &str) -> u32 {
        if self.heaps.len() as u32 >= HEAP_ID_COUNT {
            return 0;
        }
        let id = HEAP_ID_BASE + self.heaps.len() as u32;
        self.heaps.push(HeapInfo::new(id, name));
        id
    }

    /// Port of `_sys_heap_delete_heap`.  The C++ side unconditionally
    /// calls `idm::remove` and returns `CELL_OK` regardless of whether
    /// the heap existed; mirror that forgiving behaviour.
    pub fn delete_heap(&mut self, heap: u32) -> Result<(), CellError> {
        if let Some(pos) = self.heaps.iter().position(|h| h.id == heap) {
            self.heaps.swap_remove(pos);
        }
        Ok(())
    }

    /// Port of `_sys_heap_malloc`.  Returns a pseudo-address.
    ///
    /// # Errors
    /// Never errors; 0 is returned when `next_addr` would overflow
    /// (matching a failed `vm::alloc`).
    pub fn heap_malloc(&mut self, _heap: u32, size: u32) -> u32 {
        if size == 0 {
            return 0; // vm::alloc(0) is a guest bug; mirror with 0.
        }
        let addr = self.next_addr;
        let Some(next) = self.next_addr.checked_add(size) else {
            return 0;
        };
        self.next_addr = next;
        if let Some(h) = self.heaps.iter_mut().find(|h| h.id == _heap) {
            h.live_allocations += 1;
        }
        addr
    }

    /// Port of `_sys_heap_memalign`.  Aligns `next_addr` up to
    /// `max(align, 0x10000)` before allocating.
    #[must_use]
    pub fn heap_memalign(&mut self, heap: u32, align: u32, size: u32) -> u32 {
        let align = align.max(MEMALIGN_MIN);
        let mask = align - 1;
        let Some(aligned) = self.next_addr.checked_add(mask) else { return 0 };
        let aligned = aligned & !mask;
        self.next_addr = aligned;
        self.heap_malloc(heap, size)
    }

    /// Port of `_sys_heap_free`.  Firmware unconditionally returns
    /// `CELL_OK` after `vm::dealloc`.  We decrement the liveness
    /// counter if the caller supplies a valid heap id — purely for
    /// test introspection.
    pub fn heap_free(&mut self, heap: u32, _addr: u32) -> Result<(), CellError> {
        if let Some(h) = self.heaps.iter_mut().find(|h| h.id == heap) {
            h.live_allocations = h.live_allocations.saturating_sub(1);
        }
        Ok(())
    }

    // ---- stubs: `todo()` in C++, always CELL_OK ----

    /// Port of `_sys_heap_alloc_heap_memory` (stub).
    pub fn alloc_heap_memory(&self) -> Result<(), CellError> { Ok(()) }
    /// Port of `_sys_heap_get_mallinfo` (stub).
    pub fn get_mallinfo(&self) -> Result<(), CellError> { Ok(()) }
    /// Port of `_sys_heap_get_total_free_size` (stub).
    pub fn get_total_free_size(&self) -> Result<(), CellError> { Ok(()) }
    /// Port of `_sys_heap_stats` (stub).
    pub fn stats(&self) -> Result<(), CellError> { Ok(()) }

    #[must_use]
    pub fn heap_count(&self) -> usize { self.heaps.len() }

    #[must_use]
    pub fn heap_name(&self, id: u32) -> Option<&str> {
        self.heaps.iter().find(|h| h.id == id).map(|h| h.name.as_str())
    }

    #[must_use]
    pub fn heap_live_allocations(&self, id: u32) -> Option<u32> {
        self.heaps.iter().find(|h| h.id == id).map(|h| h.live_allocations)
    }
}

// =====================================================================
// sys_spinlock.cpp — busy-wait primitives
// =====================================================================

/// Port of `sys_spinlock_initialize`.  C++ body is
/// `if (*lock) *lock = 0;` — this only writes when the slot was already
/// non-zero (preserving any in-flight race).  Mirror the conditional
/// write.
pub fn spinlock_initialize(lock: &mut u32) {
    if *lock != SPINLOCK_FREE_SENTINEL {
        *lock = SPINLOCK_FREE_SENTINEL;
    }
}

/// Port of `sys_spinlock_trylock` — single-shot exchange.
/// Returns `Ok(())` on success, `Err(CELL_EBUSY)` on contention.
///
/// # Errors
/// [`CELL_EBUSY`] if another holder currently owns the lock.
pub fn spinlock_trylock(lock: &mut u32) -> Result<(), CellError> {
    if *lock != SPINLOCK_FREE_SENTINEL {
        return Err(CELL_EBUSY);
    }
    *lock = SPINLOCK_HELD_SENTINEL;
    Ok(())
}

/// Port of `sys_spinlock_lock`.  The C++ side busy-waits until the
/// exchange succeeds; the Rust mirror supports a `max_iters` cap so
/// tests can observe the acquisition order without hanging.  The C++
/// side also honours thread-stop events — callers that must support
/// cancellation should check [`CpuStop::Pending`] between ticks.
///
/// # Errors
/// [`CELL_EBUSY`] if the lock could not be acquired within `max_iters`.
pub fn spinlock_lock_with_cap(lock: &mut u32, max_iters: u32) -> Result<(), CellError> {
    for _ in 0..max_iters {
        if spinlock_trylock(lock).is_ok() {
            return Ok(());
        }
    }
    Err(CELL_EBUSY)
}

/// Port of `sys_spinlock_unlock`.  Unconditional clear — matches
/// `*lock = 0` in sys_spinlock.cpp:49.
pub fn spinlock_unlock(lock: &mut u32) {
    *lock = SPINLOCK_FREE_SENTINEL;
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- constants ---------------------------------------------------

    #[test]
    fn spinlock_sentinel_byte_exact() {
        assert_eq!(SPINLOCK_HELD_SENTINEL, 0xABAD_CAFE);
        assert_eq!(SPINLOCK_FREE_SENTINEL, 0);
    }

    #[test]
    fn cell_ebusy_byte_exact() {
        assert_eq!(CELL_EBUSY.0, 0x8001_000A);
    }

    #[test]
    fn heap_constants_byte_exact() {
        assert_eq!(HEAP_ID_BASE, 1);
        assert_eq!(HEAP_ID_COUNT, 1023);
        assert_eq!(HEAP_ID_LIMIT, 1024);
        assert_eq!(MEMALIGN_MIN, 0x10000);
    }

    // ---- sys_heap ---------------------------------------------------

    #[test]
    fn create_heap_assigns_id_base_first() {
        let mut h = SysHeap::new();
        let id = h.create_heap("game_heap");
        assert_eq!(id, 1);
        assert_eq!(h.heap_name(id), Some("game_heap"));
    }

    #[test]
    fn create_heap_ids_are_monotonic() {
        let mut h = SysHeap::new();
        assert_eq!(h.create_heap("a"), 1);
        assert_eq!(h.create_heap("b"), 2);
        assert_eq!(h.create_heap("c"), 3);
    }

    #[test]
    fn create_heap_preserves_name() {
        let mut h = SysHeap::new();
        let id = h.create_heap("music_audio_heap");
        assert_eq!(h.heap_name(id), Some("music_audio_heap"));
    }

    #[test]
    fn create_heap_returns_zero_when_full() {
        let mut h = SysHeap::new();
        for i in 0..HEAP_ID_COUNT {
            assert_ne!(h.create_heap("x"), 0, "alloc {i} should succeed");
        }
        // 1024th call exceeds the table.
        assert_eq!(h.create_heap("overflow"), 0);
    }

    #[test]
    fn delete_heap_removes_entry() {
        let mut h = SysHeap::new();
        let id = h.create_heap("a");
        h.delete_heap(id).unwrap();
        assert_eq!(h.heap_name(id), None);
    }

    #[test]
    fn delete_unknown_heap_is_still_ok() {
        // C++ idm::remove returns void; cellFn always returns CELL_OK.
        let mut h = SysHeap::new();
        assert!(h.delete_heap(9999).is_ok());
    }

    #[test]
    fn heap_malloc_returns_nonzero_address() {
        let mut h = SysHeap::new();
        let heap = h.create_heap("a");
        let addr = h.heap_malloc(heap, 1024);
        assert_ne!(addr, 0);
        assert_eq!(h.heap_live_allocations(heap), Some(1));
    }

    #[test]
    fn heap_malloc_zero_size_returns_zero() {
        let mut h = SysHeap::new();
        let heap = h.create_heap("a");
        assert_eq!(h.heap_malloc(heap, 0), 0);
    }

    #[test]
    fn heap_malloc_addresses_are_bumping() {
        let mut h = SysHeap::new();
        let heap = h.create_heap("a");
        let a = h.heap_malloc(heap, 0x1000);
        let b = h.heap_malloc(heap, 0x1000);
        assert_eq!(b, a + 0x1000);
    }

    #[test]
    fn heap_memalign_enforces_min_alignment() {
        let mut h = SysHeap::new();
        let heap = h.create_heap("a");
        // Request align=4, but firmware enforces at least 0x10000.
        let addr = h.heap_memalign(heap, 4, 0x100);
        assert_eq!(addr & 0xFFFF, 0, "address must be 64 KiB aligned");
    }

    #[test]
    fn heap_memalign_honours_larger_request() {
        let mut h = SysHeap::new();
        let heap = h.create_heap("a");
        let addr = h.heap_memalign(heap, 0x40000, 0x100);
        assert_eq!(addr & (0x40000 - 1), 0, "must be 256 KiB aligned");
    }

    #[test]
    fn heap_free_decrements_allocations() {
        let mut h = SysHeap::new();
        let heap = h.create_heap("a");
        let addr = h.heap_malloc(heap, 0x1000);
        assert_eq!(h.heap_live_allocations(heap), Some(1));
        h.heap_free(heap, addr).unwrap();
        assert_eq!(h.heap_live_allocations(heap), Some(0));
    }

    #[test]
    fn heap_free_unknown_heap_is_ok() {
        // vm::dealloc is called unconditionally; cellFn returns CELL_OK.
        let mut h = SysHeap::new();
        assert!(h.heap_free(9999, 0xDEAD).is_ok());
    }

    #[test]
    fn heap_free_saturates_at_zero() {
        let mut h = SysHeap::new();
        let heap = h.create_heap("a");
        // Free with no outstanding allocations — should not underflow.
        h.heap_free(heap, 0x1000).unwrap();
        assert_eq!(h.heap_live_allocations(heap), Some(0));
    }

    #[test]
    fn stub_entry_points_all_ok() {
        let h = SysHeap::new();
        assert!(h.alloc_heap_memory().is_ok());
        assert!(h.get_mallinfo().is_ok());
        assert!(h.get_total_free_size().is_ok());
        assert!(h.stats().is_ok());
    }

    #[test]
    fn heap_count_tracks_lifecycle() {
        let mut h = SysHeap::new();
        assert_eq!(h.heap_count(), 0);
        let a = h.create_heap("a");
        let _b = h.create_heap("b");
        assert_eq!(h.heap_count(), 2);
        h.delete_heap(a).unwrap();
        assert_eq!(h.heap_count(), 1);
    }

    // ---- sys_spinlock -----------------------------------------------

    #[test]
    fn spinlock_initialize_clears_held_lock() {
        let mut lock: u32 = 0xABAD_CAFE;
        spinlock_initialize(&mut lock);
        assert_eq!(lock, 0);
    }

    #[test]
    fn spinlock_initialize_leaves_free_lock_alone() {
        let mut lock: u32 = 0;
        spinlock_initialize(&mut lock);
        assert_eq!(lock, 0);
    }

    #[test]
    fn spinlock_initialize_clears_arbitrary_nonzero() {
        let mut lock: u32 = 0xDEAD_BEEF;
        spinlock_initialize(&mut lock);
        assert_eq!(lock, 0);
    }

    #[test]
    fn spinlock_trylock_acquires_free_lock() {
        let mut lock: u32 = 0;
        spinlock_trylock(&mut lock).unwrap();
        assert_eq!(lock, SPINLOCK_HELD_SENTINEL);
    }

    #[test]
    fn spinlock_trylock_rejects_held_lock_with_cell_ebusy() {
        let mut lock: u32 = SPINLOCK_HELD_SENTINEL;
        let err = spinlock_trylock(&mut lock).unwrap_err();
        assert_eq!(err, CELL_EBUSY);
        assert_eq!(lock, SPINLOCK_HELD_SENTINEL); // unchanged
    }

    #[test]
    fn spinlock_lock_with_cap_acquires_free_lock() {
        let mut lock: u32 = 0;
        spinlock_lock_with_cap(&mut lock, 10).unwrap();
        assert_eq!(lock, SPINLOCK_HELD_SENTINEL);
    }

    #[test]
    fn spinlock_lock_with_cap_times_out() {
        let mut lock: u32 = SPINLOCK_HELD_SENTINEL;
        assert_eq!(spinlock_lock_with_cap(&mut lock, 5).unwrap_err(), CELL_EBUSY);
    }

    #[test]
    fn spinlock_unlock_clears() {
        let mut lock: u32 = SPINLOCK_HELD_SENTINEL;
        spinlock_unlock(&mut lock);
        assert_eq!(lock, 0);
    }

    #[test]
    fn spinlock_unlock_even_if_free_is_still_free() {
        let mut lock: u32 = 0;
        spinlock_unlock(&mut lock);
        assert_eq!(lock, 0);
    }

    // ---- full smoke lifecycle ---------------------------------------

    #[test]
    fn full_sys_heap_lifecycle_smoke() {
        let mut h = SysHeap::new();
        // 1. Game creates two heaps.
        let gfx = h.create_heap("gfx_heap");
        let aud = h.create_heap("audio_heap");
        assert_eq!((gfx, aud), (1, 2));

        // 2. Allocate some memory out of each.
        let a = h.heap_malloc(gfx, 0x2000);
        let b = h.heap_malloc(aud, 0x100);
        let c = h.heap_memalign(gfx, 0x20000, 0x1000);
        assert_ne!(a, 0);
        assert_ne!(b, 0);
        assert_ne!(c, 0);
        // c should be 128 KiB aligned.
        assert_eq!(c & (0x20000 - 1), 0);

        // 3. Free one — live count drops for the right heap.
        assert_eq!(h.heap_live_allocations(gfx), Some(2));
        h.heap_free(gfx, a).unwrap();
        assert_eq!(h.heap_live_allocations(gfx), Some(1));

        // 4. Delete both heaps.
        h.delete_heap(gfx).unwrap();
        h.delete_heap(aud).unwrap();
        assert_eq!(h.heap_count(), 0);

        // 5. Stubs stay green.
        h.alloc_heap_memory().unwrap();
        h.stats().unwrap();
    }

    #[test]
    fn full_spinlock_lifecycle_smoke() {
        let mut lock: u32 = 0xDEAD_BEEF;
        // Initial state is garbage from BSS — initialize clears.
        spinlock_initialize(&mut lock);
        assert_eq!(lock, 0);

        // Acquire via trylock.
        spinlock_trylock(&mut lock).unwrap();
        assert_eq!(lock, SPINLOCK_HELD_SENTINEL);

        // Second trylock contends.
        assert_eq!(spinlock_trylock(&mut lock).unwrap_err(), CELL_EBUSY);

        // Release.
        spinlock_unlock(&mut lock);
        assert_eq!(lock, 0);

        // Bounded lock works after unlock.
        spinlock_lock_with_cap(&mut lock, 100).unwrap();
        assert_eq!(lock, SPINLOCK_HELD_SENTINEL);
    }
}
