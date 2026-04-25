//! `rpcs3-hle-sys-mempool` — PS3 memory pool manager HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/sys_mempool.cpp`.  The firmware module
//! exposes a classic fixed-block memory pool guarded by an internal
//! `sys_mutex` + `sys_cond`; `allocate_block` blocks until a block is
//! available, while `try_allocate_block` returns null on empty.  Blocks
//! are carved from a caller-supplied `chunk` region of size
//! `chunk_size`, split into `chunk_size / block_size` blocks with
//! alignment constraints validated up-front.
//!
//! ## Entry points covered
//!
//! | HLE function                     | Rust wrapper                    |
//! |----------------------------------|---------------------------------|
//! | `sys_mempool_create`             | [`SysMempool::create`]          |
//! | `sys_mempool_destroy`            | [`SysMempool::destroy`]         |
//! | `sys_mempool_allocate_block`     | [`SysMempool::allocate_block`]  |
//! | `sys_mempool_try_allocate_block` | [`SysMempool::try_allocate_block`] |
//! | `sys_mempool_free_block`         | [`SysMempool::free_block`]      |
//! | `sys_mempool_get_count`          | [`SysMempool::get_count`]       |

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with LV2's sys_cell errors.
// =====================================================================

/// `CELL_EINVAL` — invalid argument (from `rpcs3-emu-types`).
pub const CELL_EINVAL: CellError = CellError(0x8001_0002);

// =====================================================================
// Constants — byte-exact with sys_mempool.cpp:16-18
// =====================================================================

/// `memory_pool_t::id_base`.
pub const MEMPOOL_ID_BASE: u32 = 1;

/// `memory_pool_t::id_count`.
pub const MEMPOOL_ID_COUNT: u32 = 1023;

/// Exclusive upper bound of the pool-id range.
pub const MEMPOOL_ID_LIMIT: u32 = MEMPOOL_ID_BASE + MEMPOOL_ID_COUNT;

/// Alignment the firmware silently substitutes when callers pass `0` or
/// `2` — see sys_mempool.cpp:41-44
/// (`if (ralignment == 0 || ralignment == 2) alignment = 4;`).
pub const DEFAULT_ALIGNMENT: u64 = 4;

/// Required alignment of the `chunk` pointer (see sys_mempool.cpp:53
/// `chunk.aligned(8)`).
pub const CHUNK_PTR_ALIGN: u32 = 8;

// =====================================================================
// Memory-pool record
// =====================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryPool {
    pub id: u32,
    pub chunk: u32,
    pub chunk_size: u64,
    pub block_size: u64,
    pub ralignment: u64,
    pub free_blocks: Vec<u32>,
    /// Count of outstanding allocations — starts at zero because every
    /// block is free at creation time.
    pub in_flight: u64,
}

impl MemoryPool {
    #[must_use]
    pub fn num_blocks(&self) -> u64 {
        self.chunk_size / self.block_size
    }
}

// =====================================================================
// Manager
// =====================================================================

#[derive(Debug, Default, Clone)]
pub struct SysMempool {
    pools: Vec<MemoryPool>,
}

impl SysMempool {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Port of `sys_mempool_create`.  Validates arguments, carves the
    /// chunk into blocks, and returns the pool id.
    ///
    /// # Errors
    /// * [`CELL_EINVAL`] if `block_size > chunk_size`.
    /// * [`CELL_EINVAL`] if effective alignment is not a power of two.
    /// * [`CELL_EINVAL`] if `chunk` is not 8-byte aligned.
    /// * [`CELL_EINVAL`] if the pool table is full (`idm::make` fail).
    pub fn create(
        &mut self,
        chunk: u32,
        chunk_size: u64,
        block_size: u64,
        ralignment: u64,
    ) -> Result<u32, CellError> {
        // Port of sys_mempool.cpp:35-56.
        if block_size > chunk_size {
            return Err(CELL_EINVAL);
        }
        // Firmware remaps 0/2 to 4 silently.
        let alignment = if ralignment == 0 || ralignment == 2 { DEFAULT_ALIGNMENT } else { ralignment };
        // Alignment must be power of two (and non-zero — firmware checks
        // via `(alignment & (alignment-1)) != 0` which returns "bad" for
        // 0 via signed underflow handling; our effective alignment is
        // always >= 4, so no zero issue).
        if (alignment & (alignment.wrapping_sub(1))) != 0 {
            return Err(CELL_EINVAL);
        }
        if chunk % CHUNK_PTR_ALIGN != 0 {
            return Err(CELL_EINVAL);
        }
        if self.pools.len() as u32 >= MEMPOOL_ID_COUNT {
            // Mirror `idm::make` failure — firmware dereferences a null
            // pointer next, but we surface as EINVAL.
            return Err(CELL_EINVAL);
        }
        if block_size == 0 {
            return Err(CELL_EINVAL);
        }

        let id = MEMPOOL_ID_BASE + self.pools.len() as u32;
        let num_blocks = chunk_size / block_size;
        let mut free_blocks = Vec::with_capacity(num_blocks as usize);
        for i in 0..num_blocks {
            // chunk + i * block_size — truncates to u32 exactly like
            // C++ `static_cast<u32>(block_size)`.
            let addr = chunk.wrapping_add((i as u32).wrapping_mul(block_size as u32));
            free_blocks.push(addr);
        }
        self.pools.push(MemoryPool {
            id,
            chunk,
            chunk_size,
            block_size,
            ralignment: alignment,
            free_blocks,
            in_flight: 0,
        });
        Ok(id)
    }

    /// Port of `sys_mempool_destroy`.  The firmware logs an error if the
    /// pool is unknown; we mirror that tolerance by returning `Ok(())`
    /// either way — callers can pre-check with [`SysMempool::get`].
    pub fn destroy(&mut self, mempool: u32) -> Result<(), CellError> {
        if let Some(pos) = self.pools.iter().position(|p| p.id == mempool) {
            self.pools.swap_remove(pos);
        }
        Ok(())
    }

    /// Port of `sys_mempool_free_block`.  Firmware adds the block to the
    /// free list and signals the condvar.
    ///
    /// # Errors
    /// * [`CELL_EINVAL`] if the pool id is unknown.
    /// * [`CELL_EINVAL`] if the block address is past the chunk end
    ///   (`block > chunk + chunk_size`).
    pub fn free_block(&mut self, mempool: u32, block: u32) -> Result<(), CellError> {
        let pool = self.pools.iter_mut().find(|p| p.id == mempool)
            .ok_or(CELL_EINVAL)?;
        // sys_mempool.cpp:146 uses `>` strictly; preserve that.
        if u64::from(block) > u64::from(pool.chunk) + pool.chunk_size {
            return Err(CELL_EINVAL);
        }
        pool.free_blocks.push(block);
        pool.in_flight = pool.in_flight.saturating_sub(1);
        Ok(())
    }

    /// Port of `sys_mempool_get_count`.  Returns the current number of
    /// free blocks.  C++ returns `u64`; unknown pool returns `EINVAL`
    /// cast to `u64` (a huge value) — we keep that wire shape.
    #[must_use]
    pub fn get_count(&self, mempool: u32) -> u64 {
        match self.pools.iter().find(|p| p.id == mempool) {
            Some(pool) => pool.free_blocks.len() as u64,
            None => u64::from(CELL_EINVAL.0),
        }
    }

    /// Port of `sys_mempool_try_allocate_block` — non-blocking pop.
    /// Returns `0` (vm::null) if the pool is unknown or empty.
    #[must_use]
    pub fn try_allocate_block(&mut self, mempool: u32) -> u32 {
        let Some(pool) = self.pools.iter_mut().find(|p| p.id == mempool) else {
            return 0;
        };
        if let Some(addr) = pool.free_blocks.pop() {
            pool.in_flight += 1;
            addr
        } else {
            0
        }
    }

    /// Port of `sys_mempool_allocate_block`.  C++ blocks on a condvar
    /// when empty; the Rust port cannot block a CPU thread, so the
    /// wrapper either (a) returns a block immediately or (b) returns
    /// `0` plus [`AllocateOutcome::WouldBlock`] so higher layers can
    /// orchestrate the wait.
    #[must_use]
    pub fn allocate_block(&mut self, mempool: u32) -> AllocateOutcome {
        let Some(pool) = self.pools.iter_mut().find(|p| p.id == mempool) else {
            return AllocateOutcome::UnknownPool;
        };
        if let Some(addr) = pool.free_blocks.pop() {
            pool.in_flight += 1;
            AllocateOutcome::Allocated(addr)
        } else {
            AllocateOutcome::WouldBlock
        }
    }

    #[must_use]
    pub fn get(&self, mempool: u32) -> Option<&MemoryPool> {
        self.pools.iter().find(|p| p.id == mempool)
    }

    #[must_use]
    pub fn pool_count(&self) -> usize { self.pools.len() }
}

/// Outcome of [`SysMempool::allocate_block`] — distinguishes the three
/// observable states of the firmware `allocate_block` call: a block was
/// handed out, no block was available right now (caller would block on
/// the condvar), or the pool id was invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocateOutcome {
    Allocated(u32),
    WouldBlock,
    UnknownPool,
}

impl AllocateOutcome {
    #[must_use]
    pub fn allocated(self) -> Option<u32> {
        match self {
            Self::Allocated(a) => Some(a),
            _ => None,
        }
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pool(m: &mut SysMempool) -> u32 {
        // 256-byte chunk at 0x1000_0000 carved into 16-byte blocks.
        m.create(0x1000_0000, 256, 16, 16).unwrap()
    }

    // ---- constants ---------------------------------------------------

    #[test]
    fn constants_byte_exact() {
        assert_eq!(MEMPOOL_ID_BASE, 1);
        assert_eq!(MEMPOOL_ID_COUNT, 1023);
        assert_eq!(MEMPOOL_ID_LIMIT, 1024);
        assert_eq!(DEFAULT_ALIGNMENT, 4);
        assert_eq!(CHUNK_PTR_ALIGN, 8);
        assert_eq!(CELL_EINVAL.0, 0x8001_0002);
    }

    // ---- sys_mempool_create validations ------------------------------

    #[test]
    fn create_rejects_block_larger_than_chunk() {
        let mut m = SysMempool::new();
        assert_eq!(m.create(0x1000_0000, 16, 32, 16).unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn create_zero_block_size_is_invalid() {
        let mut m = SysMempool::new();
        assert_eq!(m.create(0x1000_0000, 256, 0, 16).unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn create_ralignment_zero_becomes_four() {
        let mut m = SysMempool::new();
        let id = m.create(0x1000_0000, 256, 16, 0).unwrap();
        assert_eq!(m.get(id).unwrap().ralignment, 4);
    }

    #[test]
    fn create_ralignment_two_becomes_four() {
        let mut m = SysMempool::new();
        let id = m.create(0x1000_0000, 256, 16, 2).unwrap();
        assert_eq!(m.get(id).unwrap().ralignment, 4);
    }

    #[test]
    fn create_non_power_of_two_alignment_invalid() {
        let mut m = SysMempool::new();
        assert_eq!(m.create(0x1000_0000, 256, 16, 3).unwrap_err(), CELL_EINVAL);
        assert_eq!(m.create(0x1000_0000, 256, 16, 7).unwrap_err(), CELL_EINVAL);
        assert_eq!(m.create(0x1000_0000, 256, 16, 127).unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn create_power_of_two_alignments_accepted() {
        for align in [4u64, 8, 16, 32, 64, 128, 256] {
            let mut m = SysMempool::new();
            assert!(m.create(0x1000_0000, 256, 16, align).is_ok(), "{align}");
        }
    }

    #[test]
    fn create_misaligned_chunk_invalid() {
        let mut m = SysMempool::new();
        // 0x1000_0003 is not 8-aligned.
        assert_eq!(m.create(0x1000_0003, 256, 16, 16).unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn create_8_aligned_chunk_accepted() {
        let mut m = SysMempool::new();
        assert!(m.create(0x1000_0008, 256, 16, 16).is_ok());
    }

    #[test]
    fn create_assigns_id_base_first() {
        let mut m = SysMempool::new();
        let id = make_pool(&mut m);
        assert_eq!(id, 1);
    }

    #[test]
    fn create_ids_are_monotonic() {
        let mut m = SysMempool::new();
        let a = make_pool(&mut m);
        let b = make_pool(&mut m);
        let c = make_pool(&mut m);
        assert_eq!((a, b, c), (1, 2, 3));
    }

    #[test]
    fn create_at_table_limit_fails() {
        let mut m = SysMempool::new();
        for _ in 0..MEMPOOL_ID_COUNT {
            make_pool(&mut m);
        }
        assert_eq!(
            m.create(0x1000_0000, 256, 16, 16).unwrap_err(),
            CELL_EINVAL,
        );
    }

    // ---- block initialization ---------------------------------------

    #[test]
    fn create_splits_chunk_into_blocks() {
        let mut m = SysMempool::new();
        let id = make_pool(&mut m);
        let pool = m.get(id).unwrap();
        assert_eq!(pool.num_blocks(), 16); // 256 / 16
        assert_eq!(pool.free_blocks.len(), 16);
    }

    #[test]
    fn create_blocks_have_correct_addresses() {
        let mut m = SysMempool::new();
        let id = m.create(0x1000_0000, 64, 16, 16).unwrap();
        let pool = m.get(id).unwrap();
        // Blocks should be at chunk + i*block_size:
        // 0x1000_0000, 0x1000_0010, 0x1000_0020, 0x1000_0030.
        assert_eq!(pool.free_blocks.len(), 4);
        assert!(pool.free_blocks.contains(&0x1000_0000));
        assert!(pool.free_blocks.contains(&0x1000_0010));
        assert!(pool.free_blocks.contains(&0x1000_0020));
        assert!(pool.free_blocks.contains(&0x1000_0030));
    }

    // ---- allocate / try_allocate ------------------------------------

    #[test]
    fn allocate_block_returns_allocated_from_non_empty() {
        let mut m = SysMempool::new();
        let id = make_pool(&mut m);
        match m.allocate_block(id) {
            AllocateOutcome::Allocated(a) => assert_ne!(a, 0),
            other => panic!("expected Allocated, got {other:?}"),
        }
    }

    #[test]
    fn allocate_block_would_block_on_empty() {
        let mut m = SysMempool::new();
        let id = make_pool(&mut m);
        let total = m.get(id).unwrap().free_blocks.len();
        // Drain every block.
        for _ in 0..total {
            assert!(m.allocate_block(id).allocated().is_some());
        }
        assert!(matches!(m.allocate_block(id), AllocateOutcome::WouldBlock));
    }

    #[test]
    fn allocate_block_unknown_pool_returns_unknown_pool() {
        let mut m = SysMempool::new();
        assert!(matches!(m.allocate_block(999), AllocateOutcome::UnknownPool));
    }

    #[test]
    fn try_allocate_block_returns_zero_on_unknown() {
        let mut m = SysMempool::new();
        assert_eq!(m.try_allocate_block(42), 0);
    }

    #[test]
    fn try_allocate_block_returns_zero_on_empty() {
        let mut m = SysMempool::new();
        let id = make_pool(&mut m);
        let total = m.get(id).unwrap().free_blocks.len();
        for _ in 0..total {
            assert_ne!(m.try_allocate_block(id), 0);
        }
        assert_eq!(m.try_allocate_block(id), 0);
    }

    #[test]
    fn try_allocate_returns_non_null_on_non_empty() {
        let mut m = SysMempool::new();
        let id = make_pool(&mut m);
        assert_ne!(m.try_allocate_block(id), 0);
    }

    #[test]
    fn allocate_decrements_free_count() {
        let mut m = SysMempool::new();
        let id = make_pool(&mut m);
        let before = m.get_count(id);
        m.allocate_block(id).allocated().unwrap();
        let after = m.get_count(id);
        assert_eq!(after, before - 1);
    }

    #[test]
    fn allocate_increments_in_flight() {
        let mut m = SysMempool::new();
        let id = make_pool(&mut m);
        m.allocate_block(id).allocated().unwrap();
        m.allocate_block(id).allocated().unwrap();
        assert_eq!(m.get(id).unwrap().in_flight, 2);
    }

    // ---- free_block --------------------------------------------------

    #[test]
    fn free_block_unknown_pool_is_invalid() {
        let mut m = SysMempool::new();
        assert_eq!(m.free_block(42, 0x1000_0000).unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn free_block_past_chunk_end_is_invalid() {
        let mut m = SysMempool::new();
        let id = make_pool(&mut m);
        // chunk = 0x1000_0000, chunk_size = 256 → last legal is 0x1000_00FF;
        // block beyond 0x1000_0100 is rejected per C++ `>` check.
        let past_end = 0x1000_0000 + 256 + 1;
        assert_eq!(m.free_block(id, past_end).unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn free_block_at_chunk_end_is_ok() {
        let mut m = SysMempool::new();
        let id = make_pool(&mut m);
        // Exactly at chunk + chunk_size should succeed (the C++ check
        // uses `>`, not `>=`).
        let at_end = 0x1000_0000 + 256;
        m.free_block(id, at_end).unwrap();
    }

    #[test]
    fn free_block_adds_to_pool() {
        let mut m = SysMempool::new();
        let id = make_pool(&mut m);
        let before = m.get_count(id);
        m.free_block(id, 0x1000_0010).unwrap();
        assert_eq!(m.get_count(id), before + 1);
    }

    #[test]
    fn free_block_saturates_in_flight_counter() {
        let mut m = SysMempool::new();
        let id = make_pool(&mut m);
        // Free without having allocated — in_flight stays 0.
        m.free_block(id, 0x1000_0020).unwrap();
        assert_eq!(m.get(id).unwrap().in_flight, 0);
    }

    // ---- destroy ----------------------------------------------------

    #[test]
    fn destroy_removes_pool() {
        let mut m = SysMempool::new();
        let id = make_pool(&mut m);
        m.destroy(id).unwrap();
        assert!(m.get(id).is_none());
    }

    #[test]
    fn destroy_unknown_is_ok() {
        // C++ logs an error but returns void; mirror with Ok(()).
        let mut m = SysMempool::new();
        assert!(m.destroy(999).is_ok());
    }

    // ---- get_count ---------------------------------------------------

    #[test]
    fn get_count_fresh_pool_equals_num_blocks() {
        let mut m = SysMempool::new();
        let id = make_pool(&mut m);
        assert_eq!(m.get_count(id), 16);
    }

    #[test]
    fn get_count_unknown_pool_returns_einval_cast() {
        let m = SysMempool::new();
        // C++ returns CELL_EINVAL as u64 when pool is missing.
        assert_eq!(m.get_count(42), u64::from(CELL_EINVAL.0));
    }

    // ---- full smoke -------------------------------------------------

    #[test]
    fn full_mempool_lifecycle_smoke() {
        let mut m = SysMempool::new();
        // Create a pool with 8 blocks of 32 bytes = 256-byte chunk.
        let id = m.create(0x1000_0000, 256, 32, 8).unwrap();
        assert_eq!(m.get_count(id), 8);

        // Allocate all 8 blocks — order is stack-like (LIFO).
        let mut allocated = alloc::vec::Vec::new();
        for _ in 0..8 {
            allocated.push(m.allocate_block(id).allocated().unwrap());
        }
        assert_eq!(m.get_count(id), 0);
        assert_eq!(m.get(id).unwrap().in_flight, 8);

        // Try-allocate on empty pool yields null.
        assert_eq!(m.try_allocate_block(id), 0);

        // Allocate on empty pool would block.
        assert!(matches!(m.allocate_block(id), AllocateOutcome::WouldBlock));

        // Free one block — count goes back up, in_flight down.
        m.free_block(id, allocated[0]).unwrap();
        assert_eq!(m.get_count(id), 1);
        assert_eq!(m.get(id).unwrap().in_flight, 7);

        // Free a block past chunk end — EINVAL.
        let past_end = 0x1000_0000 + 256 + 32;
        assert_eq!(m.free_block(id, past_end).unwrap_err(), CELL_EINVAL);

        // Destroy removes the pool.
        m.destroy(id).unwrap();
        assert!(m.get(id).is_none());

        // Count on dead pool gets the firmware error sentinel.
        assert_eq!(m.get_count(id), u64::from(CELL_EINVAL.0));
    }
}
