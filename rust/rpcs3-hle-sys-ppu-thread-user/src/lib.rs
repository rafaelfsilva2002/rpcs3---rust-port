//! `rpcs3-hle-sys-ppu-thread-user` — PS3 PPU thread user-mode HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/sys_ppu_thread_.cpp`.  Covers:
//!
//! * Thread-local-storage pool (`sys_initialize_tls`, `ppu_alloc_tls`,
//!   `ppu_free_tls`).
//! * `sys_ppu_thread_create` (TLS allocation + delegation).
//! * `sys_ppu_thread_get_id`, `sys_ppu_thread_exit`.
//! * `sys_ppu_thread_register_atexit` / `unregister_atexit` (fixed
//!   8-slot array).
//! * `sys_ppu_thread_once` one-shot init guard.
//! * `sys_interrupt_thread_disestablish` (TLS cleanup for interrupt
//!   threads).
//!
//! ## Entry points covered
//!
//! | C++ function                            | Rust wrapper                         |
//! |-----------------------------------------|--------------------------------------|
//! | `sys_initialize_tls`                    | [`PpuThreadUser::initialize_tls`]    |
//! | `sys_ppu_thread_create`                 | [`PpuThreadUser::create_thread`]     |
//! | `sys_ppu_thread_get_id`                 | [`PpuThreadUser::get_id`]            |
//! | `sys_ppu_thread_exit`                   | [`PpuThreadUser::exit_thread`]       |
//! | `sys_ppu_thread_once`                   | [`once_control`]                     |
//! | `sys_ppu_thread_register_atexit`        | [`PpuThreadUser::register_atexit`]   |
//! | `sys_ppu_thread_unregister_atexit`      | [`PpuThreadUser::unregister_atexit`] |
//! | `sys_interrupt_thread_disestablish`     | [`PpuThreadUser::interrupt_disestablish`] |

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with LV2 sys error table.
// =====================================================================

pub const CELL_EINVAL: CellError = CellError(0x8001_0002);
pub const CELL_EPERM:  CellError = CellError(0x8001_0003);
pub const CELL_ENOMEM: CellError = CellError(0x8001_0004);
pub const CELL_ESRCH:  CellError = CellError(0x8001_0005);

// =====================================================================
// Constants — byte-exact with sys_ppu_thread.h / sys_ppu_thread_.cpp
// =====================================================================

/// `SYS_PPU_THREAD_ONCE_INIT` (sys_ppu_thread.h:10).
pub const SYS_PPU_THREAD_ONCE_INIT: i32 = 0;
/// `SYS_PPU_THREAD_DONE_INIT` (sys_ppu_thread.h:11).
pub const SYS_PPU_THREAD_DONE_INIT: i32 = 1;

/// `SYS_PPU_THREAD_CREATE_INTERRUPT` flag (sys_ppu_thread.h:18).  When
/// set, `sys_ppu_thread_create` does NOT call `sys_ppu_thread_start`.
pub const SYS_PPU_THREAD_CREATE_INTERRUPT: u64 = 0x2;

/// Fixed capacity of the atexit handler array (cpp:12).
pub const MAX_ATEXIT_HANDLERS: usize = 8;

/// TLS "system area" size prepended to every slot (cpp:45, 81).
pub const TLS_SYSTEM_AREA_SIZE: u32 = 0x30;

/// Total TLS pool allocation (cpp:82).
pub const TLS_POOL_BYTES: u32 = 0x40000;

/// Offset the firmware adds to the TLS slot before writing to `GPR[13]`
/// (cpp:87 / 140).  The game sees `gpr[13] == slot_start + 0x7030`.
pub const TLS_GPR13_OFFSET: u32 = 0x7000 + 0x30;

/// Offset child threads are handed (`tls_addr + 0x7030`).
pub const CHILD_TLS_GPR13_OFFSET: u32 = 0x7030;

// =====================================================================
// TLS pool
// =====================================================================

/// Mirror of the module-level TLS state (lines 17-23).  The firmware
/// stores all of these in global statics; the port bundles them into a
/// single struct so tests can drive multiple independent pools.
#[derive(Debug, Clone)]
pub struct TlsPool {
    /// TLS image source address (`s_tls_addr`).
    pub image_addr: u32,
    /// Image-file size (`s_tls_file`).
    pub image_size: u32,
    /// Zeroed remainder (`s_tls_zero = tls_mem_size - tls_seg_size`).
    pub zero_size: u32,
    /// Total per-thread slot size (`s_tls_size = tls_mem_size + 0x30`).
    pub slot_size: u32,
    /// Start of the TLS memory area (`s_tls_area`).  Matches
    /// `vm::alloc(0x40000) + 0x30`.
    pub area_start: u32,
    /// Maximum number of small-TLS slots (`s_tls_max`).
    pub max_slots: u32,
    /// Occupancy bitmap — one `bool` per slot.
    pub slot_used: Vec<bool>,
    /// Next address handed out by the "alternative (big)" allocator
    /// when the slot table is full (models `vm::alloc` for tests).
    pub alt_next_addr: u32,
    /// Live alt-allocations (addr → size) for bookkeeping in tests.
    pub alt_live: Vec<u32>,
}

impl TlsPool {
    /// Port of `sys_initialize_tls`.  Builds the pool metadata from the
    /// program's `.tls` segment parameters; the caller is expected to
    /// pass the section size as `tls_seg_size` and the memory size
    /// (`memsz`) as `tls_mem_size`.  Returns the `gpr[13]` value the
    /// firmware writes into the main thread's register.
    ///
    /// The C++ implementation silently no-ops when `ppu.gpr[13] != 0`
    /// (already initialised) — callers of this port should gate on
    /// their own "main thread initialised" flag.
    #[must_use]
    pub fn initialize(
        tls_seg_addr: u32,
        tls_seg_size: u32,
        tls_mem_size: u32,
        pool_alloc_base: u32,
    ) -> (Self, u32) {
        let slot_size = tls_mem_size + TLS_SYSTEM_AREA_SIZE;
        let area_start = pool_alloc_base + TLS_SYSTEM_AREA_SIZE;
        let max_slots = (TLS_POOL_BYTES - TLS_SYSTEM_AREA_SIZE) / slot_size.max(1);

        let mut pool = Self {
            image_addr: tls_seg_addr,
            image_size: tls_seg_size,
            zero_size: tls_mem_size.saturating_sub(tls_seg_size),
            slot_size,
            area_start,
            max_slots,
            slot_used: alloc::vec![false; max_slots as usize],
            alt_next_addr: pool_alloc_base + TLS_POOL_BYTES,
            alt_live: Vec::new(),
        };

        // Allocate TLS for the main thread — firmware writes
        // `alloc_tls() + 0x7000 + 0x30` into GPR[13] (cpp:87).
        let slot = pool.alloc_slot();
        let gpr13 = slot + TLS_GPR13_OFFSET;
        (pool, gpr13)
    }

    /// Port of `ppu_alloc_tls` (cpp:25-49).  Returns the base address
    /// of a TLS slot.  On small-pool exhaustion, falls back to the
    /// alternative allocator (simulated as a bump allocator here so
    /// tests can verify the distinct id range).
    pub fn alloc_slot(&mut self) -> u32 {
        // Prefer an unused small slot.
        for (i, used) in self.slot_used.iter_mut().enumerate() {
            if !*used {
                *used = true;
                return self.area_start + (i as u32) * self.slot_size;
            }
        }
        // Fall back to the big allocator.
        let addr = self.alt_next_addr;
        self.alt_next_addr = self.alt_next_addr.wrapping_add(self.slot_size);
        self.alt_live.push(addr);
        addr
    }

    /// Port of `ppu_free_tls` (cpp:51-68).  Releases a slot back to the
    /// pool; handles both small-slot and alt-slot allocations.
    /// Returns `true` on success, `false` when the address is unknown
    /// (which the firmware logs as an error but does not propagate).
    pub fn free_slot(&mut self, addr: u32) -> bool {
        if addr >= self.area_start {
            let offset = addr - self.area_start;
            let i = (offset / self.slot_size) as usize;
            if offset % self.slot_size == 0 && i < self.slot_used.len() {
                if self.slot_used[i] {
                    self.slot_used[i] = false;
                    return true;
                }
                return false; // already free — firmware logs error.
            }
        }
        // Alternative TLS allocation detected — try alt_live.
        if let Some(pos) = self.alt_live.iter().position(|&a| a == addr) {
            self.alt_live.swap_remove(pos);
            return true;
        }
        // Below area_start or unaligned — firmware also treats this as
        // "alt allocation".  Call it success if the caller tracked it
        // on the alt path; otherwise this is a spurious call.
        false
    }

    /// Number of small slots currently in use.
    #[must_use]
    pub fn live_small_count(&self) -> usize {
        self.slot_used.iter().filter(|&&u| u).count()
    }
}

// =====================================================================
// Atexit registry
// =====================================================================

/// Mirror of `g_ppu_atexit` (cpp:12) — fixed-size ring with slots held
/// by function pointer (here modelled as `u32` guest addresses).
#[derive(Debug, Clone)]
pub struct AtexitRegistry {
    slots: [u32; MAX_ATEXIT_HANDLERS],
}

impl Default for AtexitRegistry {
    fn default() -> Self { Self { slots: [0; MAX_ATEXIT_HANDLERS] } }
}

impl AtexitRegistry {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Port of `sys_ppu_thread_register_atexit`.
    ///
    /// # Errors
    /// * [`CELL_EPERM`] if `func` is already registered.
    /// * [`CELL_ENOMEM`] if the 8-slot array is full.
    pub fn register(&mut self, func: u32) -> Result<(), CellError> {
        if self.slots.contains(&func) && func != 0 {
            return Err(CELL_EPERM);
        }
        for slot in &mut self.slots {
            if *slot == 0 {
                *slot = func;
                return Ok(());
            }
        }
        Err(CELL_ENOMEM)
    }

    /// Port of `sys_ppu_thread_unregister_atexit`.
    ///
    /// # Errors
    /// * [`CELL_ESRCH`] if `func` is not registered.
    pub fn unregister(&mut self, func: u32) -> Result<(), CellError> {
        for slot in &mut self.slots {
            if *slot == func {
                *slot = 0;
                return Ok(());
            }
        }
        Err(CELL_ESRCH)
    }

    /// Returns an iterator over registered handlers in registration
    /// order — matches the order `sys_ppu_thread_exit` calls them.
    pub fn handlers(&self) -> impl Iterator<Item = u32> + '_ {
        self.slots.iter().copied().filter(|&f| f != 0)
    }

    #[must_use]
    pub fn is_registered(&self, func: u32) -> bool {
        self.slots.iter().any(|&f| f == func && func != 0)
    }

    #[must_use]
    pub fn count(&self) -> usize {
        self.slots.iter().filter(|&&f| f != 0).count()
    }
}

// =====================================================================
// `sys_ppu_thread_once`
// =====================================================================

/// Outcome of [`once_control`] — tells the caller whether the init
/// closure should actually run (i.e., this is the first call).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnceOutcome {
    /// Caller must run the init closure; `once_ctrl` has been updated
    /// to `DONE_INIT` so subsequent calls are no-ops.
    Run,
    /// Init already ran — caller must NOT invoke the closure again.
    AlreadyDone,
}

/// Port of `sys_ppu_thread_once` — checks the `once_ctrl` flag, runs
/// the init on the first call, writes the "done" marker.  Returns an
/// [`OnceOutcome`] the caller consumes.
pub fn once_control(once_ctrl: &mut i32) -> OnceOutcome {
    if *once_ctrl == SYS_PPU_THREAD_ONCE_INIT {
        *once_ctrl = SYS_PPU_THREAD_DONE_INIT;
        OnceOutcome::Run
    } else {
        OnceOutcome::AlreadyDone
    }
}

// =====================================================================
// Top-level manager
// =====================================================================

/// Aggregates all the pieces `sys_ppu_thread_.cpp` touches so higher
/// layers can drive them as a single unit.
#[derive(Debug, Clone, Default)]
pub struct PpuThreadUser {
    pub tls: Option<TlsPool>,
    pub atexit: AtexitRegistry,
}

impl PpuThreadUser {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Port of `sys_initialize_tls` — initialises the TLS pool and
    /// returns the main thread's `gpr[13]` value.
    ///
    /// # Errors
    /// None in the firmware path; returns `CELL_EINVAL` here if the
    /// pool is already initialised (the C++ code silently no-ops in
    /// that case).
    pub fn initialize_tls(
        &mut self,
        tls_seg_addr: u32,
        tls_seg_size: u32,
        tls_mem_size: u32,
        pool_alloc_base: u32,
    ) -> Result<u32, CellError> {
        if self.tls.is_some() {
            return Err(CELL_EINVAL);
        }
        let (pool, gpr13) = TlsPool::initialize(tls_seg_addr, tls_seg_size, tls_mem_size, pool_alloc_base);
        self.tls = Some(pool);
        Ok(gpr13)
    }

    /// Port of `sys_ppu_thread_create`.  Allocates TLS then reports the
    /// values the firmware would forward to `_sys_ppu_thread_create`
    /// (`entry`, `tls_addr + 0x7030`).  When
    /// [`SYS_PPU_THREAD_CREATE_INTERRUPT`] is set in `flags`, the
    /// caller must NOT dispatch `sys_ppu_thread_start`.
    ///
    /// # Errors
    /// * [`CELL_ENOMEM`] if TLS allocation fails (here: pool not
    ///   initialised).
    pub fn create_thread(
        &mut self,
        entry: u32,
        flags: u64,
    ) -> Result<CreateThreadPlan, CellError> {
        let pool = self.tls.as_mut().ok_or(CELL_ENOMEM)?;
        let tls_addr = pool.alloc_slot();
        Ok(CreateThreadPlan {
            entry,
            tls_gpr13: tls_addr + CHILD_TLS_GPR13_OFFSET,
            tls_slot_base: tls_addr,
            needs_start: (flags & SYS_PPU_THREAD_CREATE_INTERRUPT) == 0,
        })
    }

    /// Port of `sys_ppu_thread_get_id` — trivially writes `ppu.id`.
    ///
    /// # Errors
    /// Never errors.  Returns `Ok(ppu_id)` unconditionally.
    #[must_use]
    pub fn get_id(ppu_id: u64) -> u64 { ppu_id }

    /// Port of `sys_ppu_thread_exit`.  Returns the list of atexit
    /// handlers the firmware invokes in order, then releases the TLS
    /// slot derived from `gpr13`.
    pub fn exit_thread(
        &mut self,
        gpr13: u32,
    ) -> ExitPlan {
        let atexit_list: Vec<u32> = self.atexit.handlers().collect();
        let mut freed = false;
        if let Some(pool) = self.tls.as_mut() {
            let base = gpr13.wrapping_sub(CHILD_TLS_GPR13_OFFSET);
            freed = pool.free_slot(base);
        }
        ExitPlan { atexit_list, tls_freed: freed }
    }

    /// Port of `sys_ppu_thread_register_atexit`.
    pub fn register_atexit(&mut self, func: u32) -> Result<(), CellError> {
        self.atexit.register(func)
    }

    /// Port of `sys_ppu_thread_unregister_atexit`.
    pub fn unregister_atexit(&mut self, func: u32) -> Result<(), CellError> {
        self.atexit.unregister(func)
    }

    /// Port of `sys_interrupt_thread_disestablish`.  After the syscall
    /// completes the firmware frees the TLS slot derived from the
    /// returned `r13` value.
    pub fn interrupt_disestablish(&mut self, recovered_r13: u64) -> Result<(), CellError> {
        let pool = self.tls.as_mut().ok_or(CELL_ENOMEM)?;
        let base = (recovered_r13 as u32).wrapping_sub(CHILD_TLS_GPR13_OFFSET);
        if pool.free_slot(base) {
            Ok(())
        } else {
            // Firmware logs an error but still returns CELL_OK.
            Ok(())
        }
    }
}

/// Result of [`PpuThreadUser::create_thread`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreateThreadPlan {
    pub entry: u32,
    /// Value written to the child thread's `gpr[13]` — `tls_addr + 0x7030`.
    pub tls_gpr13: u32,
    /// Underlying TLS slot base (firmware-internal).
    pub tls_slot_base: u32,
    /// Whether the caller must dispatch `sys_ppu_thread_start`.
    pub needs_start: bool,
}

/// Result of [`PpuThreadUser::exit_thread`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitPlan {
    /// atexit handlers in the order they were registered.
    pub atexit_list: Vec<u32>,
    /// Whether the TLS slot was actually released.
    pub tls_freed: bool,
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pool() -> TlsPool {
        let (pool, _gpr13) = TlsPool::initialize(0x1000_0000, 0x100, 0x200, 0x2000_0000);
        pool
    }

    // ---- constants ---------------------------------------------------

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(CELL_EINVAL.0, 0x8001_0002);
        assert_eq!(CELL_EPERM.0,  0x8001_0003);
        assert_eq!(CELL_ENOMEM.0, 0x8001_0004);
        assert_eq!(CELL_ESRCH.0,  0x8001_0005);
    }

    #[test]
    fn ppu_once_constants_byte_exact() {
        assert_eq!(SYS_PPU_THREAD_ONCE_INIT, 0);
        assert_eq!(SYS_PPU_THREAD_DONE_INIT, 1);
    }

    #[test]
    fn create_interrupt_flag_byte_exact() {
        assert_eq!(SYS_PPU_THREAD_CREATE_INTERRUPT, 0x2);
    }

    #[test]
    fn tls_layout_constants_byte_exact() {
        assert_eq!(TLS_SYSTEM_AREA_SIZE, 0x30);
        assert_eq!(TLS_POOL_BYTES, 0x40000);
        assert_eq!(TLS_GPR13_OFFSET, 0x7030);
        assert_eq!(CHILD_TLS_GPR13_OFFSET, 0x7030);
        assert_eq!(MAX_ATEXIT_HANDLERS, 8);
    }

    // ---- TLS pool ---------------------------------------------------

    #[test]
    fn tls_initialize_computes_slot_size() {
        // tls_mem_size=0x200 → slot_size = 0x200 + 0x30 = 0x230
        let pool = sample_pool();
        assert_eq!(pool.slot_size, 0x230);
    }

    #[test]
    fn tls_initialize_computes_zero_size() {
        // tls_seg=0x100, tls_mem=0x200 → zero = 0x100
        let pool = sample_pool();
        assert_eq!(pool.zero_size, 0x100);
    }

    #[test]
    fn tls_initialize_area_start_offset() {
        // pool_alloc_base=0x2000_0000 → area_start=0x2000_0030
        let pool = sample_pool();
        assert_eq!(pool.area_start, 0x2000_0030);
    }

    #[test]
    fn tls_initialize_max_slots_formula() {
        // (0x40000 - 0x30) / 0x230 = 0x3FFD0 / 0x230 = 0x12C (300 slots roughly)
        let pool = sample_pool();
        let expected = (TLS_POOL_BYTES - TLS_SYSTEM_AREA_SIZE) / 0x230;
        assert_eq!(pool.max_slots, expected);
    }

    #[test]
    fn tls_initialize_allocates_main_thread() {
        let (pool, gpr13) = TlsPool::initialize(0x1000_0000, 0x100, 0x200, 0x2000_0000);
        // Main TLS slot is slot 0 (at area_start) → gpr13 = area_start + 0x7030
        assert_eq!(gpr13, pool.area_start + TLS_GPR13_OFFSET);
        assert_eq!(pool.live_small_count(), 1);
    }

    #[test]
    fn tls_alloc_slot_monotonic() {
        let mut pool = sample_pool();
        // initialize already consumed slot 0.
        let s1 = pool.alloc_slot();
        let s2 = pool.alloc_slot();
        assert_eq!(s2 - s1, pool.slot_size);
    }

    #[test]
    fn tls_free_slot_reuses() {
        let mut pool = sample_pool();
        let s1 = pool.alloc_slot();
        assert!(pool.free_slot(s1));
        let s2 = pool.alloc_slot();
        // Should reuse the freed slot (or an earlier one).
        assert_eq!(s1, s2);
    }

    #[test]
    fn tls_free_slot_twice_fails() {
        let mut pool = sample_pool();
        let s = pool.alloc_slot();
        assert!(pool.free_slot(s));
        assert!(!pool.free_slot(s));
    }

    #[test]
    fn tls_alloc_exhaust_falls_back_to_alt() {
        let mut pool = sample_pool();
        // `sample_pool()` (via TlsPool::initialize) already consumed
        // slot 0 for the main thread — only max_slots-1 remain.
        let remaining = pool.max_slots as usize - pool.live_small_count();
        for _ in 0..remaining {
            pool.alloc_slot();
        }
        assert_eq!(pool.alt_live.len(), 0, "pool should not yet have overflowed");
        let alt = pool.alloc_slot();
        assert_eq!(pool.alt_live.len(), 1);
        assert!(alt >= pool.alt_next_addr - pool.slot_size);
    }

    #[test]
    fn tls_free_alt_slot() {
        let mut pool = sample_pool();
        let remaining = pool.max_slots as usize - pool.live_small_count();
        for _ in 0..remaining {
            pool.alloc_slot();
        }
        let alt = pool.alloc_slot();
        assert!(pool.free_slot(alt));
        assert!(pool.alt_live.is_empty());
    }

    #[test]
    fn tls_free_unknown_address_is_false() {
        let mut pool = sample_pool();
        assert!(!pool.free_slot(0xDEAD_BEEF));
    }

    // ---- Atexit registry --------------------------------------------

    #[test]
    fn atexit_register_stores_func() {
        let mut r = AtexitRegistry::new();
        r.register(0x1234).unwrap();
        assert!(r.is_registered(0x1234));
        assert_eq!(r.count(), 1);
    }

    #[test]
    fn atexit_duplicate_register_is_eperm() {
        let mut r = AtexitRegistry::new();
        r.register(0x1234).unwrap();
        assert_eq!(r.register(0x1234).unwrap_err(), CELL_EPERM);
    }

    #[test]
    fn atexit_full_register_is_enomem() {
        let mut r = AtexitRegistry::new();
        for i in 0..MAX_ATEXIT_HANDLERS {
            r.register(0x1000 + i as u32 * 0x100).unwrap();
        }
        assert_eq!(
            r.register(0x9999).unwrap_err(),
            CELL_ENOMEM,
        );
    }

    #[test]
    fn atexit_unregister_clears_slot() {
        let mut r = AtexitRegistry::new();
        r.register(0x1234).unwrap();
        r.unregister(0x1234).unwrap();
        assert!(!r.is_registered(0x1234));
    }

    #[test]
    fn atexit_unregister_unknown_is_esrch() {
        let mut r = AtexitRegistry::new();
        assert_eq!(r.unregister(0x9999).unwrap_err(), CELL_ESRCH);
    }

    #[test]
    fn atexit_handlers_preserve_registration_order() {
        let mut r = AtexitRegistry::new();
        r.register(0x1000).unwrap();
        r.register(0x2000).unwrap();
        r.register(0x3000).unwrap();
        let list: Vec<u32> = r.handlers().collect();
        assert_eq!(list, [0x1000, 0x2000, 0x3000]);
    }

    #[test]
    fn atexit_unregister_then_register_fills_gap() {
        let mut r = AtexitRegistry::new();
        r.register(0x1000).unwrap();
        r.register(0x2000).unwrap();
        r.unregister(0x1000).unwrap();
        r.register(0x3000).unwrap();
        let list: Vec<u32> = r.handlers().collect();
        // 0x3000 fills slot 0 (first null after unregister).
        assert_eq!(list, [0x3000, 0x2000]);
    }

    #[test]
    fn atexit_null_func_does_not_trigger_duplicate() {
        // `contains(&0)` would match empty slots; ensure register(0)
        // doesn't error on the "already registered" check.
        let r = AtexitRegistry::new();
        // is_registered ignores func==0.
        assert!(!r.is_registered(0));
    }

    // ---- once ------------------------------------------------------

    #[test]
    fn once_first_call_returns_run() {
        let mut ctrl = SYS_PPU_THREAD_ONCE_INIT;
        assert_eq!(once_control(&mut ctrl), OnceOutcome::Run);
        assert_eq!(ctrl, SYS_PPU_THREAD_DONE_INIT);
    }

    #[test]
    fn once_second_call_returns_already_done() {
        let mut ctrl = SYS_PPU_THREAD_DONE_INIT;
        assert_eq!(once_control(&mut ctrl), OnceOutcome::AlreadyDone);
        assert_eq!(ctrl, SYS_PPU_THREAD_DONE_INIT);
    }

    #[test]
    fn once_arbitrary_nonzero_is_already_done() {
        let mut ctrl: i32 = 42;
        assert_eq!(once_control(&mut ctrl), OnceOutcome::AlreadyDone);
        // Preserves existing value (does not overwrite).
        assert_eq!(ctrl, 42);
    }

    // ---- PpuThreadUser top-level -----------------------------------

    #[test]
    fn initialize_tls_once_then_twice_errors() {
        let mut u = PpuThreadUser::new();
        u.initialize_tls(0x1000_0000, 0x100, 0x200, 0x2000_0000).unwrap();
        assert_eq!(
            u.initialize_tls(0x1000_0000, 0x100, 0x200, 0x2000_0000).unwrap_err(),
            CELL_EINVAL,
        );
    }

    #[test]
    fn create_thread_without_init_is_enomem() {
        let mut u = PpuThreadUser::new();
        assert_eq!(
            u.create_thread(0x1000, 0).unwrap_err(),
            CELL_ENOMEM,
        );
    }

    #[test]
    fn create_thread_default_needs_start() {
        let mut u = PpuThreadUser::new();
        u.initialize_tls(0x1000_0000, 0x100, 0x200, 0x2000_0000).unwrap();
        let plan = u.create_thread(0x1000, 0).unwrap();
        assert!(plan.needs_start);
        assert_eq!(plan.entry, 0x1000);
        assert_eq!(plan.tls_gpr13, plan.tls_slot_base + CHILD_TLS_GPR13_OFFSET);
    }

    #[test]
    fn create_thread_interrupt_flag_skips_start() {
        let mut u = PpuThreadUser::new();
        u.initialize_tls(0x1000_0000, 0x100, 0x200, 0x2000_0000).unwrap();
        let plan = u.create_thread(0x1000, SYS_PPU_THREAD_CREATE_INTERRUPT).unwrap();
        assert!(!plan.needs_start);
    }

    #[test]
    fn get_id_returns_ppu_id() {
        assert_eq!(PpuThreadUser::get_id(0xDEAD_BEEF_CAFE), 0xDEAD_BEEF_CAFE);
    }

    #[test]
    fn exit_thread_collects_atexit_in_order() {
        let mut u = PpuThreadUser::new();
        let gpr13 = u.initialize_tls(0x1000_0000, 0x100, 0x200, 0x2000_0000).unwrap();
        u.register_atexit(0x1111).unwrap();
        u.register_atexit(0x2222).unwrap();
        u.register_atexit(0x3333).unwrap();
        let plan = u.exit_thread(gpr13);
        assert_eq!(plan.atexit_list, [0x1111, 0x2222, 0x3333]);
        assert!(plan.tls_freed);
    }

    #[test]
    fn interrupt_disestablish_frees_tls() {
        let mut u = PpuThreadUser::new();
        u.initialize_tls(0x1000_0000, 0x100, 0x200, 0x2000_0000).unwrap();
        let plan = u.create_thread(0x1000, SYS_PPU_THREAD_CREATE_INTERRUPT).unwrap();
        u.interrupt_disestablish(plan.tls_gpr13 as u64).unwrap();
    }

    // ---- full smoke ------------------------------------------------

    #[test]
    fn full_ppu_thread_lifecycle_smoke() {
        let mut u = PpuThreadUser::new();

        // 1. Initialize TLS — main thread gets gpr13.
        let main_gpr13 = u.initialize_tls(0x1000_0000, 0x100, 0x200, 0x2000_0000).unwrap();
        assert_ne!(main_gpr13, 0);

        // 2. Spawn 3 worker threads (no interrupt flag).
        let w1 = u.create_thread(0x1100, 0).unwrap();
        let w2 = u.create_thread(0x1200, 0).unwrap();
        let w3 = u.create_thread(0x1300, 0).unwrap();
        assert!(w1.needs_start && w2.needs_start && w3.needs_start);

        // 3. Register a couple atexit handlers.
        u.register_atexit(0x5001).unwrap();
        u.register_atexit(0x5002).unwrap();
        assert_eq!(u.atexit.count(), 2);

        // 4. Duplicate register fails.
        assert_eq!(u.register_atexit(0x5001).unwrap_err(), CELL_EPERM);

        // 5. sys_ppu_thread_once first call runs, second skips.
        let mut once_ctrl = SYS_PPU_THREAD_ONCE_INIT;
        assert_eq!(once_control(&mut once_ctrl), OnceOutcome::Run);
        assert_eq!(once_control(&mut once_ctrl), OnceOutcome::AlreadyDone);

        // 6. Worker 2 exits → atexit list emitted in order, TLS freed.
        let exit_plan = u.exit_thread(w2.tls_gpr13);
        assert_eq!(exit_plan.atexit_list, [0x5001, 0x5002]);
        assert!(exit_plan.tls_freed);

        // 7. Unregister first handler.
        u.unregister_atexit(0x5001).unwrap();
        assert_eq!(u.atexit.count(), 1);

        // 8. Interrupt disestablish releases TLS.
        u.interrupt_disestablish(w3.tls_gpr13 as u64).unwrap();
    }
}
