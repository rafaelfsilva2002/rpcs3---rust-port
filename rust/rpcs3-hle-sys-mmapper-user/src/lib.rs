//! `rpcs3-hle-sys-mmapper-user` — PS3 memory mapper user-mode HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/sys_mmapper_.cpp` — the user-mode
//! shim that games call for shared memory allocation.  Each of the 5
//! entry points is a thin wrapper over the `sys_mmapper_*_shared_memory`
//! syscalls; two of them (`allocate_memory` and
//! `allocate_memory_from_container`) inject the
//! `SYS_MMAPPER_NO_SHM_KEY` sentinel so the underlying syscall knows
//! this is an anonymous allocation.
//!
//! ## Entry points covered
//!
//! | C++ function                                    | Rust wrapper                         |
//! |-------------------------------------------------|--------------------------------------|
//! | `sys_mmapper_allocate_memory`                   | [`SysMmapperUser::allocate_memory`]  |
//! | `sys_mmapper_allocate_memory_from_container`    | [`SysMmapperUser::allocate_from_container`] |
//! | `sys_mmapper_map_memory`                        | [`SysMmapperUser::map_memory`]       |
//! | `sys_mmapper_unmap_memory`                      | [`SysMmapperUser::unmap_memory`]     |
//! | `sys_mmapper_free_memory`                       | [`SysMmapperUser::free_memory`]      |

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes
// =====================================================================

pub const CELL_EINVAL:  CellError = CellError(0x8001_0002);
pub const CELL_ESRCH:   CellError = CellError(0x8001_0005);
pub const CELL_EFAULT:  CellError = CellError(0x8001_000D);
pub const CELL_ENOMEM:  CellError = CellError(0x8001_0004);

// =====================================================================
// Constants — byte-exact with sys_mmapper.h:49 + sys_memory.h:29-32
// =====================================================================

/// `SYS_MMAPPER_NO_SHM_KEY = 0xffff000000000000` from sys_mmapper.h:49.
/// The firmware injects this sentinel into
/// `sys_mmapper_allocate_shared_memory` when the caller invoked
/// `sys_mmapper_allocate_memory` (no explicit SHM key).
pub const SYS_MMAPPER_NO_SHM_KEY: u64 = 0xFFFF_0000_0000_0000;

pub const SYS_MEMORY_PAGE_SIZE_4K:   u64 = 0x100;
pub const SYS_MEMORY_PAGE_SIZE_64K:  u64 = 0x200;
pub const SYS_MEMORY_PAGE_SIZE_1M:   u64 = 0x400;
pub const SYS_MEMORY_PAGE_SIZE_MASK: u64 = 0xF00;

// =====================================================================
// Page-size helpers
// =====================================================================

/// Translate the `flags & SYS_MEMORY_PAGE_SIZE_MASK` bits into a byte
/// size — mirrors sys_memory.cpp:124-127.
///
/// * `SYS_MEMORY_PAGE_SIZE_1M`  → `0x100000` (1 MiB)
/// * `SYS_MEMORY_PAGE_SIZE_64K` → `0x10000`  (64 KiB)
/// * anything else              → returns `None` (caller surfaces EINVAL)
#[must_use]
pub fn flags_to_page_size(flags: u64) -> Option<u32> {
    match flags & SYS_MEMORY_PAGE_SIZE_MASK {
        SYS_MEMORY_PAGE_SIZE_1M  => Some(0x10_0000),
        SYS_MEMORY_PAGE_SIZE_64K => Some(0x1_0000),
        _ => None,
    }
}

// =====================================================================
// Request / response structs
// =====================================================================

/// Arguments forwarded from the user-side wrapper into the underlying
/// `sys_mmapper_allocate_shared_memory` syscall.  Exposed for tests
/// that need to verify the SHM-key injection behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocateSharedRequest {
    pub shm_key: u64,
    pub size: u32,
    pub flags: u64,
}

/// Same, but routed through a memory container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AllocateSharedFromContainerRequest {
    pub shm_key: u64,
    pub size: u32,
    pub container_id: u32,
    pub flags: u64,
}

/// Descriptor of a live shared-memory allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedMem {
    pub mem_id: u32,
    pub size: u32,
    pub flags: u64,
    pub shm_key: u64,
    pub container_id: Option<u32>,
    pub mapped_addr: Option<u32>,
}

// =====================================================================
// Manager — observable state of the firmware's user-mode shim
// =====================================================================

#[derive(Debug, Default, Clone)]
pub struct SysMmapperUser {
    allocs: Vec<SharedMem>,
    next_id: u32,
}

impl SysMmapperUser {
    #[must_use]
    pub fn new() -> Self {
        // `idm::make<lv2_memory>` hands out ids starting at 0x80010000
        // in the real firmware; we use 1 for deterministic tests.
        Self { allocs: Vec::new(), next_id: 1 }
    }

    /// Port of `sys_mmapper_allocate_memory` (sys_mmapper_.cpp:7-12).
    /// Delegates to the underlying allocator using
    /// [`SYS_MMAPPER_NO_SHM_KEY`] as the SHM key.  The Rust port
    /// simulates a successful allocation when `flags_to_page_size`
    /// returns a known size; otherwise emits `CELL_EINVAL` early.
    ///
    /// # Errors
    /// * [`CELL_EFAULT`] if `mem_id_ptr_valid` is false.
    /// * [`CELL_EINVAL`] if page-size bits are unknown.
    pub fn allocate_memory(
        &mut self,
        size: u32,
        flags: u64,
        mem_id_ptr_valid: bool,
    ) -> Result<(u32, AllocateSharedRequest), CellError> {
        if !mem_id_ptr_valid {
            return Err(CELL_EFAULT);
        }
        let req = AllocateSharedRequest {
            shm_key: SYS_MMAPPER_NO_SHM_KEY,
            size,
            flags,
        };
        let page_size = flags_to_page_size(flags).ok_or(CELL_EINVAL)?;
        if size == 0 || size % page_size != 0 {
            return Err(CELL_EINVAL);
        }
        let id = self.bump_id()?;
        self.allocs.push(SharedMem {
            mem_id: id, size, flags, shm_key: req.shm_key,
            container_id: None, mapped_addr: None,
        });
        Ok((id, req))
    }

    /// Port of `sys_mmapper_allocate_memory_from_container`.
    ///
    /// # Errors
    /// Same as [`Self::allocate_memory`], plus [`CELL_ESRCH`] if the
    /// caller provides `container_id == 0` (the firmware checks
    /// container existence downstream; we validate the id surface up
    /// front).
    pub fn allocate_from_container(
        &mut self,
        size: u32,
        container_id: u32,
        flags: u64,
        mem_id_ptr_valid: bool,
    ) -> Result<(u32, AllocateSharedFromContainerRequest), CellError> {
        if !mem_id_ptr_valid {
            return Err(CELL_EFAULT);
        }
        if container_id == 0 {
            return Err(CELL_ESRCH);
        }
        let req = AllocateSharedFromContainerRequest {
            shm_key: SYS_MMAPPER_NO_SHM_KEY,
            size,
            container_id,
            flags,
        };
        let page_size = flags_to_page_size(flags).ok_or(CELL_EINVAL)?;
        if size == 0 || size % page_size != 0 {
            return Err(CELL_EINVAL);
        }
        let id = self.bump_id()?;
        self.allocs.push(SharedMem {
            mem_id: id, size, flags, shm_key: req.shm_key,
            container_id: Some(container_id), mapped_addr: None,
        });
        Ok((id, req))
    }

    /// Port of `sys_mmapper_map_memory`.  Pure delegation in the C++
    /// port (cpp:21-26) — we record the mapping on our shadow table.
    ///
    /// # Errors
    /// * [`CELL_ESRCH`] if `mem_id` is unknown.
    /// * [`CELL_EINVAL`] if `addr` is already occupied by another
    ///   mapping in our shadow table (the real syscall is more lenient;
    ///   this is the fail-fast variant).
    pub fn map_memory(&mut self, addr: u32, mem_id: u32, flags: u64) -> Result<(), CellError> {
        if self.allocs.iter().any(|a| a.mapped_addr == Some(addr) && a.mem_id != mem_id) {
            return Err(CELL_EINVAL);
        }
        let slot = self.allocs.iter_mut().find(|a| a.mem_id == mem_id)
            .ok_or(CELL_ESRCH)?;
        slot.mapped_addr = Some(addr);
        // flags for map_memory (alignment / protection) are forwarded
        // verbatim; we don't otherwise interpret them.
        let _ = flags;
        Ok(())
    }

    /// Port of `sys_mmapper_unmap_memory`.  Writes the original
    /// `mem_id` into `*mem_id` (matches cpp:28-33 signature).
    ///
    /// # Errors
    /// * [`CELL_EFAULT`] if `mem_id_ptr_valid` is false.
    /// * [`CELL_EINVAL`] if no allocation is mapped at `addr`.
    pub fn unmap_memory(
        &mut self,
        addr: u32,
        mem_id_ptr_valid: bool,
    ) -> Result<u32, CellError> {
        if !mem_id_ptr_valid {
            return Err(CELL_EFAULT);
        }
        let slot = self.allocs.iter_mut().find(|a| a.mapped_addr == Some(addr))
            .ok_or(CELL_EINVAL)?;
        let id = slot.mem_id;
        slot.mapped_addr = None;
        Ok(id)
    }

    /// Port of `sys_mmapper_free_memory`.  Removes the allocation from
    /// the shadow table.
    ///
    /// # Errors
    /// * [`CELL_ESRCH`] if `mem_id` is unknown.
    /// * [`CELL_EINVAL`] if the allocation is still mapped to an
    ///   address (guest leaked the mapping).
    pub fn free_memory(&mut self, mem_id: u32) -> Result<(), CellError> {
        let pos = self.allocs.iter().position(|a| a.mem_id == mem_id)
            .ok_or(CELL_ESRCH)?;
        if self.allocs[pos].mapped_addr.is_some() {
            return Err(CELL_EINVAL);
        }
        self.allocs.swap_remove(pos);
        Ok(())
    }

    // ---- helpers --------------------------------------------------

    fn bump_id(&mut self) -> Result<u32, CellError> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or(CELL_ENOMEM)?;
        Ok(id)
    }

    #[must_use]
    pub fn get(&self, mem_id: u32) -> Option<&SharedMem> {
        self.allocs.iter().find(|a| a.mem_id == mem_id)
    }

    #[must_use]
    pub fn len(&self) -> usize { self.allocs.len() }

    #[must_use]
    pub fn is_empty(&self) -> bool { self.allocs.is_empty() }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- constants ---------------------------------------------------

    #[test]
    fn shm_key_sentinel_byte_exact() {
        // sys_mmapper.h:49 — `0xffff000000000000ull`.
        assert_eq!(SYS_MMAPPER_NO_SHM_KEY, 0xFFFF_0000_0000_0000);
    }

    #[test]
    fn page_size_constants_byte_exact() {
        assert_eq!(SYS_MEMORY_PAGE_SIZE_4K,   0x100);
        assert_eq!(SYS_MEMORY_PAGE_SIZE_64K,  0x200);
        assert_eq!(SYS_MEMORY_PAGE_SIZE_1M,   0x400);
        assert_eq!(SYS_MEMORY_PAGE_SIZE_MASK, 0xF00);
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(CELL_EINVAL.0, 0x8001_0002);
        assert_eq!(CELL_ESRCH.0,  0x8001_0005);
        assert_eq!(CELL_EFAULT.0, 0x8001_000D);
        assert_eq!(CELL_ENOMEM.0, 0x8001_0004);
    }

    // ---- flags_to_page_size -----------------------------------------

    #[test]
    fn flags_1m_returns_0x100000() {
        assert_eq!(flags_to_page_size(SYS_MEMORY_PAGE_SIZE_1M), Some(0x10_0000));
    }

    #[test]
    fn flags_64k_returns_0x10000() {
        assert_eq!(flags_to_page_size(SYS_MEMORY_PAGE_SIZE_64K), Some(0x1_0000));
    }

    #[test]
    fn flags_4k_is_unknown() {
        // 4K page size is not in the firmware's accepted set.
        assert_eq!(flags_to_page_size(SYS_MEMORY_PAGE_SIZE_4K), None);
    }

    #[test]
    fn flags_zero_is_unknown() {
        assert_eq!(flags_to_page_size(0), None);
    }

    #[test]
    fn flags_ignores_high_bits() {
        // Only the PROTOCOL_MASK bits are consulted.
        assert_eq!(
            flags_to_page_size(SYS_MEMORY_PAGE_SIZE_1M | 0x1000_0000),
            Some(0x10_0000),
        );
    }

    // ---- allocate_memory --------------------------------------------

    #[test]
    fn allocate_memory_null_mem_id_is_efault() {
        let mut m = SysMmapperUser::new();
        assert_eq!(
            m.allocate_memory(0x1_0000, SYS_MEMORY_PAGE_SIZE_64K, false).unwrap_err(),
            CELL_EFAULT,
        );
    }

    #[test]
    fn allocate_memory_injects_no_shm_key() {
        let mut m = SysMmapperUser::new();
        let (_id, req) = m.allocate_memory(0x1_0000, SYS_MEMORY_PAGE_SIZE_64K, true).unwrap();
        assert_eq!(req.shm_key, SYS_MMAPPER_NO_SHM_KEY);
    }

    #[test]
    fn allocate_memory_request_preserves_size_and_flags() {
        let mut m = SysMmapperUser::new();
        let (_id, req) = m.allocate_memory(0x30_0000, SYS_MEMORY_PAGE_SIZE_1M, true).unwrap();
        assert_eq!(req.size, 0x30_0000);
        assert_eq!(req.flags, SYS_MEMORY_PAGE_SIZE_1M);
    }

    #[test]
    fn allocate_memory_rejects_bad_page_flags() {
        let mut m = SysMmapperUser::new();
        assert_eq!(
            m.allocate_memory(0x1000, 0, true).unwrap_err(),
            CELL_EINVAL,
        );
    }

    #[test]
    fn allocate_memory_rejects_non_page_aligned_size() {
        let mut m = SysMmapperUser::new();
        // page is 64 KiB, size is 0x1234 → not aligned.
        assert_eq!(
            m.allocate_memory(0x1234, SYS_MEMORY_PAGE_SIZE_64K, true).unwrap_err(),
            CELL_EINVAL,
        );
    }

    #[test]
    fn allocate_memory_rejects_zero_size() {
        let mut m = SysMmapperUser::new();
        assert_eq!(
            m.allocate_memory(0, SYS_MEMORY_PAGE_SIZE_64K, true).unwrap_err(),
            CELL_EINVAL,
        );
    }

    #[test]
    fn allocate_memory_ids_are_monotonic() {
        let mut m = SysMmapperUser::new();
        let (a, _) = m.allocate_memory(0x1_0000, SYS_MEMORY_PAGE_SIZE_64K, true).unwrap();
        let (b, _) = m.allocate_memory(0x1_0000, SYS_MEMORY_PAGE_SIZE_64K, true).unwrap();
        let (c, _) = m.allocate_memory(0x1_0000, SYS_MEMORY_PAGE_SIZE_64K, true).unwrap();
        assert_eq!((a, b, c), (1, 2, 3));
    }

    #[test]
    fn allocate_memory_records_no_container() {
        let mut m = SysMmapperUser::new();
        let (id, _) = m.allocate_memory(0x1_0000, SYS_MEMORY_PAGE_SIZE_64K, true).unwrap();
        assert_eq!(m.get(id).unwrap().container_id, None);
    }

    // ---- allocate_from_container ------------------------------------

    #[test]
    fn allocate_from_container_injects_no_shm_key() {
        let mut m = SysMmapperUser::new();
        let (_id, req) = m.allocate_from_container(0x1_0000, 42, SYS_MEMORY_PAGE_SIZE_64K, true).unwrap();
        assert_eq!(req.shm_key, SYS_MMAPPER_NO_SHM_KEY);
        assert_eq!(req.container_id, 42);
    }

    #[test]
    fn allocate_from_container_zero_cid_is_esrch() {
        let mut m = SysMmapperUser::new();
        assert_eq!(
            m.allocate_from_container(0x1_0000, 0, SYS_MEMORY_PAGE_SIZE_64K, true).unwrap_err(),
            CELL_ESRCH,
        );
    }

    #[test]
    fn allocate_from_container_null_mem_id_is_efault() {
        let mut m = SysMmapperUser::new();
        assert_eq!(
            m.allocate_from_container(0x1_0000, 42, SYS_MEMORY_PAGE_SIZE_64K, false).unwrap_err(),
            CELL_EFAULT,
        );
    }

    #[test]
    fn allocate_from_container_records_container_id() {
        let mut m = SysMmapperUser::new();
        let (id, _) = m.allocate_from_container(0x1_0000, 42, SYS_MEMORY_PAGE_SIZE_64K, true).unwrap();
        assert_eq!(m.get(id).unwrap().container_id, Some(42));
    }

    // ---- map_memory -------------------------------------------------

    #[test]
    fn map_memory_records_address() {
        let mut m = SysMmapperUser::new();
        let (id, _) = m.allocate_memory(0x1_0000, SYS_MEMORY_PAGE_SIZE_64K, true).unwrap();
        m.map_memory(0x4000_0000, id, 0).unwrap();
        assert_eq!(m.get(id).unwrap().mapped_addr, Some(0x4000_0000));
    }

    #[test]
    fn map_memory_unknown_mem_id_is_esrch() {
        let mut m = SysMmapperUser::new();
        assert_eq!(
            m.map_memory(0x4000_0000, 999, 0).unwrap_err(),
            CELL_ESRCH,
        );
    }

    #[test]
    fn map_memory_conflicting_addr_is_einval() {
        let mut m = SysMmapperUser::new();
        let (a, _) = m.allocate_memory(0x1_0000, SYS_MEMORY_PAGE_SIZE_64K, true).unwrap();
        let (b, _) = m.allocate_memory(0x1_0000, SYS_MEMORY_PAGE_SIZE_64K, true).unwrap();
        m.map_memory(0x4000_0000, a, 0).unwrap();
        // Mapping a different id at the same address → EINVAL.
        assert_eq!(
            m.map_memory(0x4000_0000, b, 0).unwrap_err(),
            CELL_EINVAL,
        );
    }

    // ---- unmap_memory -----------------------------------------------

    #[test]
    fn unmap_memory_returns_mem_id() {
        let mut m = SysMmapperUser::new();
        let (id, _) = m.allocate_memory(0x1_0000, SYS_MEMORY_PAGE_SIZE_64K, true).unwrap();
        m.map_memory(0x4000_0000, id, 0).unwrap();
        let got = m.unmap_memory(0x4000_0000, true).unwrap();
        assert_eq!(got, id);
        assert_eq!(m.get(id).unwrap().mapped_addr, None);
    }

    #[test]
    fn unmap_memory_null_ptr_is_efault() {
        let mut m = SysMmapperUser::new();
        assert_eq!(
            m.unmap_memory(0x4000_0000, false).unwrap_err(),
            CELL_EFAULT,
        );
    }

    #[test]
    fn unmap_memory_unknown_addr_is_einval() {
        let mut m = SysMmapperUser::new();
        assert_eq!(
            m.unmap_memory(0x4000_0000, true).unwrap_err(),
            CELL_EINVAL,
        );
    }

    // ---- free_memory ------------------------------------------------

    #[test]
    fn free_memory_removes_allocation() {
        let mut m = SysMmapperUser::new();
        let (id, _) = m.allocate_memory(0x1_0000, SYS_MEMORY_PAGE_SIZE_64K, true).unwrap();
        m.free_memory(id).unwrap();
        assert!(m.get(id).is_none());
    }

    #[test]
    fn free_memory_unknown_is_esrch() {
        let mut m = SysMmapperUser::new();
        assert_eq!(m.free_memory(999).unwrap_err(), CELL_ESRCH);
    }

    #[test]
    fn free_memory_while_mapped_is_einval() {
        let mut m = SysMmapperUser::new();
        let (id, _) = m.allocate_memory(0x1_0000, SYS_MEMORY_PAGE_SIZE_64K, true).unwrap();
        m.map_memory(0x4000_0000, id, 0).unwrap();
        assert_eq!(m.free_memory(id).unwrap_err(), CELL_EINVAL);
    }

    // ---- full smoke --------------------------------------------------

    #[test]
    fn full_mmapper_lifecycle_smoke() {
        let mut m = SysMmapperUser::new();

        // 1. Allocate 64 KiB anonymous.
        let (anon_id, anon_req) = m.allocate_memory(0x1_0000, SYS_MEMORY_PAGE_SIZE_64K, true).unwrap();
        assert_eq!(anon_req.shm_key, SYS_MMAPPER_NO_SHM_KEY);

        // 2. Allocate 1 MiB from container 5.
        let (cont_id, cont_req) = m.allocate_from_container(
            0x10_0000,
            5,
            SYS_MEMORY_PAGE_SIZE_1M,
            true,
        ).unwrap();
        assert_eq!(cont_req.container_id, 5);
        assert_eq!(cont_req.shm_key, SYS_MMAPPER_NO_SHM_KEY);

        // 3. Map both.
        m.map_memory(0x4000_0000, anon_id, 0).unwrap();
        m.map_memory(0x5000_0000, cont_id, 0).unwrap();
        assert_eq!(m.len(), 2);

        // 4. Unmap anon.
        let back = m.unmap_memory(0x4000_0000, true).unwrap();
        assert_eq!(back, anon_id);

        // 5. Free anon succeeds; cont fails (still mapped).
        m.free_memory(anon_id).unwrap();
        assert_eq!(m.free_memory(cont_id).unwrap_err(), CELL_EINVAL);

        // 6. Unmap then free cont.
        m.unmap_memory(0x5000_0000, true).unwrap();
        m.free_memory(cont_id).unwrap();
        assert!(m.is_empty());
    }
}
