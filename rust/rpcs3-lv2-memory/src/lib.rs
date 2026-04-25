//! `rpcs3-lv2-memory` — LV2 memory management syscalls.
//!
//! Ports `rpcs3/Emu/Cell/lv2/sys_memory.cpp`. The actual allocation
//! logic (reserving address ranges, filling pages) is delegated to a
//! pluggable [`MemoryContainer`] trait — this crate only handles the
//! syscall-level validation (size alignment, flag checking, CELL_*
//! error mapping) and page-size resolution.
//!
//! ## Scope
//!
//! * `sys_memory_allocate(size, flags)` → addr
//! * `sys_memory_free(addr)` → ()
//! * `sys_memory_get_page_attribute(addr)` → `PageAttr`
//! * `sys_memory_get_user_memory_size()` → `MemoryInfo`
//! * `sys_memory_container_create(size)` → cid
//! * `sys_memory_container_destroy(cid)` → ()

use rpcs3_emu_types::CellError;

// =====================================================================
// Constants (sys_memory.h)
// =====================================================================

/// `SYS_MEMORY_PAGE_SIZE_*` flags passed to `sys_memory_allocate`.
pub const PAGE_SIZE_4K: u64 = 0x100;
pub const PAGE_SIZE_64K: u64 = 0x200;
pub const PAGE_SIZE_1M: u64 = 0x400;

// Page protection attributes returned by `sys_memory_get_page_attribute`.
pub const ATTR_PROT_EXECUTE: u64 = 0x0000_0000_0000_0001;
pub const ATTR_PROT_WRITE: u64 = 0x0000_0000_0000_0002;
pub const ATTR_PROT_READ: u64 = 0x0000_0000_0000_0004;

// =====================================================================
// Return types
// =====================================================================

/// Mirrors `sys_page_attr_t` from `sys_memory.h`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PageAttr {
    /// Attribute flags (ATTR_PROT_*).
    pub attribute: u64,
    /// Page size in bytes (0x10000 or 0x100000 in practice).
    pub page_size: u64,
    /// Access rights flags (also ATTR_PROT_*).
    pub access_right: u32,
    /// Page fault counter (always 0 in RPCS3).
    pub page_fault_ppu: u32,
}

/// Mirrors `sys_memory_info_t`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemoryInfo {
    pub total_user_memory: u64,
    pub available_user_memory: u64,
}

// =====================================================================
// Container abstraction
// =====================================================================

/// Physical memory container. Concrete implementations own actual
/// allocations; this trait exposes only the operations the syscall
/// surface needs.
pub trait MemoryContainer {
    /// Allocate `size` bytes aligned to `align`. Returns the guest
    /// base address on success, or `CellError` on failure.
    fn allocate(&mut self, size: u64, align: u32) -> Result<u32, CellError>;

    /// Free memory previously returned by [`Self::allocate`].
    /// Returns `CELL_EINVAL` if `addr` doesn't match an earlier alloc.
    fn free(&mut self, addr: u32) -> Result<(), CellError>;

    /// Return the attribute block for a page containing `addr`, or
    /// `CELL_EINVAL` if `addr` is not allocated.
    fn get_page_attribute(&self, addr: u32) -> Result<PageAttr, CellError>;

    /// Total user memory in bytes.
    fn total_bytes(&self) -> u64;

    /// Currently-available (unallocated) user memory in bytes.
    fn available_bytes(&self) -> u64;

    /// Create a child container carved out of this container's budget.
    /// Returns the child's container id.
    fn create_child(&mut self, size: u64) -> Result<u32, CellError>;

    /// Destroy a child container. Returns `CELL_ESRCH` if unknown.
    fn destroy_child(&mut self, cid: u32) -> Result<(), CellError>;
}

// =====================================================================
// Syscalls
// =====================================================================

/// `sys_memory_allocate(size, flags)` — sys_memory.cpp:111.
#[must_use]
pub fn sys_memory_allocate<C: MemoryContainer + ?Sized>(
    container: &mut C,
    size: u64,
    flags: u64,
) -> Result<u32, CellError> {
    if size == 0 {
        return Err(CellError(0x8001_0010)); // CELL_EALIGN
    }

    // Page-size flag → alignment.
    let align: u32 = match flags {
        0 => 0x10_0000,            // default 1 MB
        PAGE_SIZE_64K => 0x1_0000, // 64 KB
        PAGE_SIZE_1M => 0x10_0000, // 1 MB
        _ => return Err(CellError::EINVAL),
    };

    if size % u64::from(align) != 0 {
        return Err(CellError(0x8001_0010)); // CELL_EALIGN
    }

    container.allocate(size, align)
}

/// `sys_memory_free(addr)` — sys_memory.cpp:247.
#[must_use]
pub fn sys_memory_free<C: MemoryContainer + ?Sized>(
    container: &mut C,
    addr: u32,
) -> Result<(), CellError> {
    container.free(addr)
}

/// `sys_memory_get_page_attribute(addr)` — sys_memory.cpp:265.
#[must_use]
pub fn sys_memory_get_page_attribute<C: MemoryContainer + ?Sized>(
    container: &C,
    addr: u32,
) -> Result<PageAttr, CellError> {
    container.get_page_attribute(addr)
}

/// `sys_memory_get_user_memory_size()` — sys_memory.cpp:324.
#[must_use]
pub fn sys_memory_get_user_memory_size<C: MemoryContainer + ?Sized>(container: &C) -> MemoryInfo {
    MemoryInfo {
        total_user_memory: container.total_bytes(),
        available_user_memory: container.available_bytes(),
    }
}

/// `sys_memory_container_create(size)` — sys_memory.cpp:375.
#[must_use]
pub fn sys_memory_container_create<C: MemoryContainer + ?Sized>(
    container: &mut C,
    size: u64,
) -> Result<u32, CellError> {
    // 1 MB minimum, 1 MB alignment (matches the C++ check).
    if size == 0 || size % 0x10_0000 != 0 {
        return Err(CellError(0x8001_0010)); // CELL_EALIGN
    }
    container.create_child(size)
}

/// `sys_memory_container_destroy(cid)` — sys_memory.cpp:411.
#[must_use]
pub fn sys_memory_container_destroy<C: MemoryContainer + ?Sized>(
    container: &mut C,
    cid: u32,
) -> Result<(), CellError> {
    container.destroy_child(cid)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// In-memory container used by the tests. Tracks allocations,
    /// available budget, and child containers. Not thread-safe — this
    /// is test scaffolding, not the production implementation.
    struct TestContainer {
        total: u64,
        used: u64,
        allocations: HashMap<u32, u64>, // addr → size
        next_addr: u32,
        children: HashMap<u32, u64>, // cid → size
        next_cid: u32,
    }

    impl TestContainer {
        fn new(total: u64) -> Self {
            Self {
                total,
                used: 0,
                allocations: HashMap::new(),
                next_addr: 0x2000_0000,
                children: HashMap::new(),
                next_cid: 1,
            }
        }
    }

    impl MemoryContainer for TestContainer {
        fn allocate(&mut self, size: u64, align: u32) -> Result<u32, CellError> {
            if self.total - self.used < size {
                return Err(CellError::ENOMEM);
            }
            // Round address up to align boundary.
            let mask = u64::from(align) - 1;
            let next = (u64::from(self.next_addr) + mask) & !mask;
            let addr = next as u32;
            self.next_addr = (next + size) as u32;
            self.used += size;
            self.allocations.insert(addr, size);
            Ok(addr)
        }
        fn free(&mut self, addr: u32) -> Result<(), CellError> {
            let size = self.allocations.remove(&addr).ok_or(CellError::EINVAL)?;
            self.used -= size;
            Ok(())
        }
        fn get_page_attribute(&self, addr: u32) -> Result<PageAttr, CellError> {
            // Find the allocation containing `addr`.
            let found = self.allocations.iter().find(|(&base, &size)| {
                addr >= base && addr < base.wrapping_add(size as u32)
            });
            if found.is_none() {
                return Err(CellError::EINVAL);
            }
            Ok(PageAttr {
                attribute: ATTR_PROT_READ | ATTR_PROT_WRITE,
                page_size: 0x10_0000,
                access_right: (ATTR_PROT_READ | ATTR_PROT_WRITE) as u32,
                page_fault_ppu: 0,
            })
        }
        fn total_bytes(&self) -> u64 {
            self.total
        }
        fn available_bytes(&self) -> u64 {
            self.total - self.used
        }
        fn create_child(&mut self, size: u64) -> Result<u32, CellError> {
            if self.total - self.used < size {
                return Err(CellError::ENOMEM);
            }
            let cid = self.next_cid;
            self.next_cid += 1;
            self.children.insert(cid, size);
            self.used += size;
            Ok(cid)
        }
        fn destroy_child(&mut self, cid: u32) -> Result<(), CellError> {
            let size = self.children.remove(&cid).ok_or(CellError::ESRCH)?;
            self.used -= size;
            Ok(())
        }
    }

    // -- allocate --------------------------------------------------

    #[test]
    fn allocate_zero_size_is_ealign() {
        let mut c = TestContainer::new(0x1000_0000);
        assert_eq!(sys_memory_allocate(&mut c, 0, 0), Err(CellError(0x8001_0010)));
    }

    #[test]
    fn allocate_unknown_flag_is_einval() {
        let mut c = TestContainer::new(0x1000_0000);
        assert_eq!(
            sys_memory_allocate(&mut c, 0x1_0000, 0x999),
            Err(CellError::EINVAL)
        );
    }

    #[test]
    fn allocate_misaligned_size_is_ealign() {
        let mut c = TestContainer::new(0x1000_0000);
        // default flags=0 → 1 MB alignment; 0x12345 not multiple of 0x100000
        assert_eq!(
            sys_memory_allocate(&mut c, 0x12345, 0),
            Err(CellError(0x8001_0010))
        );
    }

    #[test]
    fn allocate_default_flags_uses_1mb_alignment() {
        let mut c = TestContainer::new(0x1000_0000);
        let addr = sys_memory_allocate(&mut c, 0x10_0000, 0).unwrap();
        assert_eq!(addr % 0x10_0000, 0);
    }

    #[test]
    fn allocate_64k_flag_uses_64k_alignment() {
        let mut c = TestContainer::new(0x1000_0000);
        let addr = sys_memory_allocate(&mut c, 0x1_0000, PAGE_SIZE_64K).unwrap();
        assert_eq!(addr % 0x1_0000, 0);
    }

    #[test]
    fn allocate_out_of_memory_is_enomem() {
        let mut c = TestContainer::new(0x10_0000); // only 1 MB
        // first alloc succeeds
        sys_memory_allocate(&mut c, 0x10_0000, 0).unwrap();
        // second request doesn't fit
        assert_eq!(
            sys_memory_allocate(&mut c, 0x10_0000, 0),
            Err(CellError::ENOMEM)
        );
    }

    // -- free ------------------------------------------------------

    #[test]
    fn free_returns_memory_to_pool() {
        let mut c = TestContainer::new(0x20_0000);
        let addr = sys_memory_allocate(&mut c, 0x10_0000, 0).unwrap();
        assert_eq!(c.available_bytes(), 0x10_0000);
        sys_memory_free(&mut c, addr).unwrap();
        assert_eq!(c.available_bytes(), 0x20_0000);
    }

    #[test]
    fn free_unknown_address_is_einval() {
        let mut c = TestContainer::new(0x10_0000);
        assert_eq!(sys_memory_free(&mut c, 0xDEAD_0000), Err(CellError::EINVAL));
    }

    // -- get_page_attribute ---------------------------------------

    #[test]
    fn get_page_attribute_of_allocated_region() {
        let mut c = TestContainer::new(0x20_0000);
        let addr = sys_memory_allocate(&mut c, 0x10_0000, 0).unwrap();
        let attr = sys_memory_get_page_attribute(&c, addr).unwrap();
        assert_eq!(attr.page_size, 0x10_0000);
        assert_ne!(attr.attribute & ATTR_PROT_READ, 0);
    }

    #[test]
    fn get_page_attribute_of_unallocated_is_einval() {
        let c = TestContainer::new(0x10_0000);
        assert_eq!(
            sys_memory_get_page_attribute(&c, 0xDEAD_0000),
            Err(CellError::EINVAL)
        );
    }

    // -- get_user_memory_size -------------------------------------

    #[test]
    fn get_user_memory_size_reflects_allocations() {
        let mut c = TestContainer::new(0x20_0000);
        let info = sys_memory_get_user_memory_size(&c);
        assert_eq!(info.total_user_memory, 0x20_0000);
        assert_eq!(info.available_user_memory, 0x20_0000);
        sys_memory_allocate(&mut c, 0x10_0000, 0).unwrap();
        let after = sys_memory_get_user_memory_size(&c);
        assert_eq!(after.available_user_memory, 0x10_0000);
    }

    // -- containers ------------------------------------------------

    #[test]
    fn container_create_returns_cid() {
        let mut c = TestContainer::new(0x20_0000);
        let cid = sys_memory_container_create(&mut c, 0x10_0000).unwrap();
        assert!(cid > 0);
    }

    #[test]
    fn container_create_rejects_non_1mb_aligned_size() {
        let mut c = TestContainer::new(0x10_0000);
        assert_eq!(
            sys_memory_container_create(&mut c, 0x12345),
            Err(CellError(0x8001_0010))
        );
    }

    #[test]
    fn container_create_rejects_zero_size() {
        let mut c = TestContainer::new(0x10_0000);
        assert_eq!(
            sys_memory_container_create(&mut c, 0),
            Err(CellError(0x8001_0010))
        );
    }

    #[test]
    fn container_destroy_round_trip() {
        let mut c = TestContainer::new(0x20_0000);
        let cid = sys_memory_container_create(&mut c, 0x10_0000).unwrap();
        sys_memory_container_destroy(&mut c, cid).unwrap();
        assert_eq!(
            sys_memory_container_destroy(&mut c, cid),
            Err(CellError::ESRCH)
        );
    }

    #[test]
    fn container_destroy_unknown_is_esrch() {
        let mut c = TestContainer::new(0x10_0000);
        assert_eq!(
            sys_memory_container_destroy(&mut c, 99),
            Err(CellError::ESRCH)
        );
    }

    // -- constants frozen ------------------------------------------

    #[test]
    fn page_size_flag_constants_frozen() {
        assert_eq!(PAGE_SIZE_4K, 0x100);
        assert_eq!(PAGE_SIZE_64K, 0x200);
        assert_eq!(PAGE_SIZE_1M, 0x400);
    }
}
