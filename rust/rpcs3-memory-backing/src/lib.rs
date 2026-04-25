//! `rpcs3-memory-backing` — page table + backing storage for PS3 memory.
//!
//! Wave 4b scope:
//!
//! * `PageTable` — 1 M atomic page-flag bytes, matching
//!   `std::array<memory_page, 0x100000000 / 4096> g_pages` (vm.h:86).
//! * `SparseBackend` — `HashMap<page_idx, [u8; 4096]>` backing used by
//!   tests and cold paths. Exposes safe `read<T>` / `write<T>`,
//!   `check_addr`, `alloc_at`, `dealloc`, `page_protect`.
//! * `ReservationTable` — 512 rows of 128-byte LL/SC reservations.
//!   Mirrors the shape of `vm::g_reservations[65536 / 128 * 64]`.
//!
//! Out of scope (Wave 4c):
//!
//! * `LinearBackend` — mmaps 4 GB of host VM and exposes raw `*mut u8`
//!   slices (for JIT). Goes behind a tight `unsafe` wall and gets
//!   `miri`/fuzz coverage.
//! * Cross-thread reservation notification (waiter wake-up).
//! * Host-level memory protection syscalls (`mprotect`/`VirtualProtect`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};

use rpcs3_memory::{
    PageFlags, ADDRESS_SPACE_SIZE, PAGE_COUNT, PAGE_SIZE, RESERVATION_BLOCK_SIZE, RESERVATION_ROWS,
};

// =====================================================================
// Errors
// =====================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// `addr + size` overflows the 32-bit guest address space.
    AddressOverflow { addr: u32, size: u32 },
    /// Access covers a page without the required flags.
    MissingFlags { addr: u32, required: PageFlags, actual: PageFlags },
    /// `alloc_at` would overlap an already-allocated page.
    AlreadyAllocated { page: u32 },
    /// `dealloc`/`page_protect` on a page that was never allocated.
    NotAllocated { page: u32 },
    /// `alloc_at` called with a non-4K-aligned address or size.
    Unaligned { addr: u32, size: u32 },
    /// `size` is zero.
    ZeroSize,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::AddressOverflow { addr, size } => {
                write!(f, "address overflow: addr=0x{addr:08x} size={size}")
            }
            Error::MissingFlags { addr, required, actual } => {
                write!(
                    f,
                    "page at 0x{addr:08x} missing required flags: required={:08b} actual={:08b}",
                    required.0, actual.0,
                )
            }
            Error::AlreadyAllocated { page } => {
                write!(f, "page {page} already allocated")
            }
            Error::NotAllocated { page } => write!(f, "page {page} not allocated"),
            Error::Unaligned { addr, size } => {
                write!(f, "unaligned alloc: addr=0x{addr:08x} size={size} (must be 4K)")
            }
            Error::ZeroSize => f.write_str("size is zero"),
        }
    }
}

impl std::error::Error for Error {}

// =====================================================================
// PageTable
// =====================================================================

/// 1 M pages × 1 byte of flags, atomically mutable.
/// Lives behind `Arc<PageTable>` when shared between CPU and JIT.
pub struct PageTable {
    pages: Vec<AtomicU8>,
}

impl core::fmt::Debug for PageTable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PageTable").field("len", &self.pages.len()).finish()
    }
}

impl Default for PageTable {
    fn default() -> Self {
        Self::new()
    }
}

impl PageTable {
    /// Fresh page table with every page flagged zero (not allocated).
    #[must_use]
    pub fn new() -> Self {
        let mut pages = Vec::with_capacity(PAGE_COUNT as usize);
        for _ in 0..PAGE_COUNT {
            pages.push(AtomicU8::new(0));
        }
        Self { pages }
    }

    /// Fast page-flag load. Returns `(allocated, flags)` matching
    /// `vm::get_addr_flags` (vm.h:98).
    #[must_use]
    pub fn get_addr_flags(&self, addr: u32) -> (bool, PageFlags) {
        let page_idx = (addr / PAGE_SIZE) as usize;
        let flags = PageFlags(self.pages[page_idx].load(Ordering::SeqCst));
        (flags.is_allocated(), flags)
    }

    /// Overwrite page flags atomically. Mostly used by allocator ops.
    pub fn set_flags(&self, page_idx: u32, flags: PageFlags) {
        self.pages[page_idx as usize].store(flags.0, Ordering::SeqCst);
    }

    /// OR new bits into the page flags (e.g. mark as allocated).
    pub fn or_flags(&self, page_idx: u32, flags: PageFlags) {
        self.pages[page_idx as usize].fetch_or(flags.0, Ordering::SeqCst);
    }

    /// AND-clear bits from the page flags.
    pub fn and_clear_flags(&self, page_idx: u32, clear: PageFlags) {
        self.pages[page_idx as usize].fetch_and(!clear.0, Ordering::SeqCst);
    }

    /// True if every page touched by `[addr, addr+size)` has the
    /// required flags plus `ALLOCATED`. Mirrors `vm::check_addr` at
    /// vm.h:81 (the multi-page path).
    pub fn check_addr(&self, addr: u32, flags: PageFlags, size: u32) -> Result<(), Error> {
        if size == 0 {
            return Err(Error::ZeroSize);
        }
        let end = addr.checked_add(size).ok_or(Error::AddressOverflow { addr, size })?;
        let first_page = addr / PAGE_SIZE;
        let last_page = (end - 1) / PAGE_SIZE;
        let required = flags.union(PageFlags::ALLOCATED);

        for page_idx in first_page..=last_page {
            let actual = PageFlags(self.pages[page_idx as usize].load(Ordering::SeqCst));
            if !actual.contains(required) {
                return Err(Error::MissingFlags {
                    addr: page_idx * PAGE_SIZE,
                    required,
                    actual,
                });
            }
        }
        Ok(())
    }

    /// Pages covered by `[addr, addr+size)` inclusive, as a page-index
    /// range. Returns `None` if the range overflows the address space.
    #[must_use]
    pub fn pages_in_range(addr: u32, size: u32) -> Option<(u32, u32)> {
        if size == 0 {
            return None;
        }
        let end = addr.checked_add(size)?;
        Some((addr / PAGE_SIZE, (end - 1) / PAGE_SIZE))
    }
}

// =====================================================================
// SparseBackend
// =====================================================================

/// Sparse page-indexed backing. Only allocated pages hold actual bytes.
/// Suitable for tests and any workload that doesn't reserve all 4 GB.
pub struct SparseBackend {
    page_table: PageTable,
    /// page_idx → backing bytes.
    pages: HashMap<u32, Box<[u8; PAGE_SIZE as usize]>>,
}

impl core::fmt::Debug for SparseBackend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SparseBackend")
            .field("allocated_pages", &self.pages.len())
            .finish()
    }
}

impl Default for SparseBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SparseBackend {
    #[must_use]
    pub fn new() -> Self {
        Self {
            page_table: PageTable::new(),
            pages: HashMap::new(),
        }
    }

    #[must_use]
    pub fn page_table(&self) -> &PageTable {
        &self.page_table
    }

    /// Allocate `size` bytes at `addr`. Both must be 4 KB aligned.
    /// Fails if any covered page is already allocated.
    pub fn alloc_at(&mut self, addr: u32, size: u32, flags: PageFlags) -> Result<(), Error> {
        if size == 0 {
            return Err(Error::ZeroSize);
        }
        if addr % PAGE_SIZE != 0 || size % PAGE_SIZE != 0 {
            return Err(Error::Unaligned { addr, size });
        }
        let end = addr.checked_add(size).ok_or(Error::AddressOverflow { addr, size })?;
        if u64::from(end) > ADDRESS_SPACE_SIZE {
            return Err(Error::AddressOverflow { addr, size });
        }

        let first = addr / PAGE_SIZE;
        let last = (end - 1) / PAGE_SIZE;

        // Dry-run: check every page first.
        for idx in first..=last {
            let (alloc, _) = self.page_table.get_addr_flags(idx * PAGE_SIZE);
            if alloc {
                return Err(Error::AlreadyAllocated { page: idx });
            }
        }

        // Commit: zero-backed pages + set flags.
        let final_flags = flags.union(PageFlags::ALLOCATED);
        for idx in first..=last {
            self.pages.insert(idx, Box::new([0u8; PAGE_SIZE as usize]));
            self.page_table.set_flags(idx, final_flags);
        }
        Ok(())
    }

    /// Free all pages covered by `[addr, addr+size)`.
    pub fn dealloc(&mut self, addr: u32, size: u32) -> Result<(), Error> {
        if addr % PAGE_SIZE != 0 || size % PAGE_SIZE != 0 {
            return Err(Error::Unaligned { addr, size });
        }
        let Some((first, last)) = PageTable::pages_in_range(addr, size) else {
            return Err(Error::ZeroSize);
        };
        for idx in first..=last {
            let (alloc, _) = self.page_table.get_addr_flags(idx * PAGE_SIZE);
            if !alloc {
                return Err(Error::NotAllocated { page: idx });
            }
        }
        for idx in first..=last {
            self.pages.remove(&idx);
            self.page_table.set_flags(idx, PageFlags::empty());
        }
        Ok(())
    }

    /// Change the protection on `[addr, addr+size)`. `flags_set` are
    /// ORed in; `flags_clear` are masked out. Mirrors `vm::page_protect`
    /// (vm.h:78), minus the `flags_test` pre-condition check.
    pub fn page_protect(
        &self,
        addr: u32,
        size: u32,
        flags_set: PageFlags,
        flags_clear: PageFlags,
    ) -> Result<(), Error> {
        let Some((first, last)) = PageTable::pages_in_range(addr, size) else {
            return Err(Error::ZeroSize);
        };
        for idx in first..=last {
            let (alloc, _) = self.page_table.get_addr_flags(idx * PAGE_SIZE);
            if !alloc {
                return Err(Error::NotAllocated { page: idx });
            }
        }
        for idx in first..=last {
            if !flags_clear.is_empty() {
                self.page_table.and_clear_flags(idx, flags_clear);
            }
            if !flags_set.is_empty() {
                self.page_table.or_flags(idx, flags_set);
            }
        }
        Ok(())
    }

    /// Read bytes from `addr`. Fails if the range isn't all readable+allocated.
    pub fn read(&self, addr: u32, dst: &mut [u8]) -> Result<(), Error> {
        let size = u32::try_from(dst.len()).map_err(|_| Error::AddressOverflow { addr, size: 0 })?;
        self.page_table.check_addr(addr, PageFlags::READABLE, size)?;

        let mut cursor = 0usize;
        let mut cur_addr = addr;
        while cursor < dst.len() {
            let page_idx = cur_addr / PAGE_SIZE;
            let page_off = (cur_addr % PAGE_SIZE) as usize;
            let page = self.pages.get(&page_idx).expect("check_addr verified allocation");
            let remaining_in_page = PAGE_SIZE as usize - page_off;
            let take = remaining_in_page.min(dst.len() - cursor);
            dst[cursor..cursor + take].copy_from_slice(&page[page_off..page_off + take]);
            cursor += take;
            cur_addr += take as u32;
        }
        Ok(())
    }

    /// Write bytes to `addr`. Fails if the range isn't all writable+allocated.
    pub fn write(&mut self, addr: u32, src: &[u8]) -> Result<(), Error> {
        let size = u32::try_from(src.len()).map_err(|_| Error::AddressOverflow { addr, size: 0 })?;
        self.page_table.check_addr(addr, PageFlags::WRITABLE, size)?;

        let mut cursor = 0usize;
        let mut cur_addr = addr;
        while cursor < src.len() {
            let page_idx = cur_addr / PAGE_SIZE;
            let page_off = (cur_addr % PAGE_SIZE) as usize;
            let page = self.pages.get_mut(&page_idx).expect("check_addr verified allocation");
            let remaining_in_page = PAGE_SIZE as usize - page_off;
            let take = remaining_in_page.min(src.len() - cursor);
            page[page_off..page_off + take].copy_from_slice(&src[cursor..cursor + take]);
            cursor += take;
            cur_addr += take as u32;
        }
        Ok(())
    }

    /// Typed read: reads `size_of::<T>()` bytes of `T` in little-endian.
    pub fn read_le<T>(&self, addr: u32) -> Result<T, Error>
    where
        T: FromLeBytes,
    {
        let mut buf = [0u8; 16];
        let size = T::SIZE;
        assert!(size <= 16, "read_le max 16 bytes; use bulk read for bigger");
        self.read(addr, &mut buf[..size])?;
        Ok(T::from_le_bytes(&buf[..size]))
    }

    /// Typed write: writes `T` in little-endian.
    pub fn write_le<T: ToLeBytes>(&mut self, addr: u32, value: T) -> Result<(), Error> {
        let mut buf = [0u8; 16];
        let size = T::SIZE;
        assert!(size <= 16, "write_le max 16 bytes");
        value.write_le_into(&mut buf[..size]);
        self.write(addr, &buf[..size])
    }

    /// Number of allocated pages (for tests / debug).
    #[must_use]
    pub fn allocated_page_count(&self) -> usize {
        self.pages.len()
    }
}

// =====================================================================
// Trivial little-endian scalar traits
// =====================================================================

pub trait FromLeBytes: Sized {
    const SIZE: usize;
    fn from_le_bytes(bytes: &[u8]) -> Self;
}

pub trait ToLeBytes {
    const SIZE: usize;
    fn write_le_into(self, out: &mut [u8]);
}

macro_rules! impl_le_scalar {
    ($ty:ty) => {
        impl FromLeBytes for $ty {
            const SIZE: usize = core::mem::size_of::<$ty>();
            fn from_le_bytes(bytes: &[u8]) -> Self {
                let arr: [u8; core::mem::size_of::<$ty>()] = bytes.try_into().unwrap();
                <$ty>::from_le_bytes(arr)
            }
        }
        impl ToLeBytes for $ty {
            const SIZE: usize = core::mem::size_of::<$ty>();
            fn write_le_into(self, out: &mut [u8]) {
                out.copy_from_slice(&<$ty>::to_le_bytes(self));
            }
        }
    };
}
impl_le_scalar!(u8);
impl_le_scalar!(u16);
impl_le_scalar!(u32);
impl_le_scalar!(u64);
impl_le_scalar!(i8);
impl_le_scalar!(i16);
impl_le_scalar!(i32);
impl_le_scalar!(i64);

// =====================================================================
// ReservationTable
// =====================================================================

/// 512 rows × (timestamp u64 + reserved flag u64) — the reservation
/// table backing LL/SC ops in `vm_reservation.h`. Simplified shape:
/// real RPCS3 packs more data per row; we keep the contract tight.
pub struct ReservationTable {
    rows: Vec<AtomicU64>,
}

impl core::fmt::Debug for ReservationTable {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ReservationTable")
            .field("rows", &self.rows.len())
            .finish()
    }
}

impl Default for ReservationTable {
    fn default() -> Self {
        Self::new()
    }
}

impl ReservationTable {
    #[must_use]
    pub fn new() -> Self {
        let mut rows = Vec::with_capacity(RESERVATION_ROWS as usize);
        for _ in 0..RESERVATION_ROWS {
            rows.push(AtomicU64::new(0));
        }
        Self { rows }
    }

    /// Load current reservation timestamp for the 128-byte block
    /// containing `addr`. Matches `vm::reservation_acquire`.
    pub fn acquire(&self, addr: u32) -> u64 {
        let row = ((addr & 0xFFFF) / RESERVATION_BLOCK_SIZE) as usize;
        self.rows[row].load(Ordering::SeqCst)
    }

    /// Bump the timestamp of the 128-byte block containing `addr`.
    /// Matches `vm::reservation_update`.
    pub fn update(&self, addr: u32) {
        let row = ((addr & 0xFFFF) / RESERVATION_BLOCK_SIZE) as usize;
        self.rows[row].fetch_add(1, Ordering::SeqCst);
    }

    /// Compare-and-set: succeeds iff timestamp hasn't changed since
    /// `prev` was observed. Returns the new timestamp on success.
    pub fn try_commit(&self, addr: u32, prev: u64) -> Result<u64, u64> {
        let row = ((addr & 0xFFFF) / RESERVATION_BLOCK_SIZE) as usize;
        self.rows[row]
            .compare_exchange(prev, prev + 1, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| prev + 1)
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- PageTable --------------------------------------------------

    #[test]
    fn page_table_starts_empty() {
        let pt = PageTable::new();
        let (alloc, flags) = pt.get_addr_flags(0);
        assert!(!alloc);
        assert_eq!(flags.0, 0);
    }

    #[test]
    fn page_table_pages_in_range_basic() {
        assert_eq!(PageTable::pages_in_range(0, 4096), Some((0, 0)));
        assert_eq!(PageTable::pages_in_range(0, 4097), Some((0, 1)));
        assert_eq!(PageTable::pages_in_range(0x1000, 0x2000), Some((1, 2)));
        assert_eq!(PageTable::pages_in_range(0, 0), None);
    }

    #[test]
    fn page_table_check_addr_requires_allocated() {
        let pt = PageTable::new();
        let err = pt.check_addr(0, PageFlags::READABLE, 1).unwrap_err();
        assert!(matches!(err, Error::MissingFlags { .. }));
    }

    // -- SparseBackend alloc/dealloc --------------------------------

    #[test]
    fn alloc_at_happy_path() {
        let mut b = SparseBackend::new();
        b.alloc_at(0x1000, 0x2000, PageFlags::READABLE | PageFlags::WRITABLE)
            .unwrap();
        assert_eq!(b.allocated_page_count(), 2);
        let (alloc, flags) = b.page_table.get_addr_flags(0x1000);
        assert!(alloc);
        assert!(flags.is_readable());
        assert!(flags.is_writable());
    }

    #[test]
    fn alloc_at_rejects_unaligned() {
        let mut b = SparseBackend::new();
        assert!(matches!(
            b.alloc_at(0x100, 0x1000, PageFlags::READABLE),
            Err(Error::Unaligned { .. })
        ));
        assert!(matches!(
            b.alloc_at(0x1000, 0x123, PageFlags::READABLE),
            Err(Error::Unaligned { .. })
        ));
    }

    #[test]
    fn alloc_at_rejects_overlap() {
        let mut b = SparseBackend::new();
        b.alloc_at(0x1000, 0x2000, PageFlags::READABLE).unwrap();
        // Overlap on page 1
        let err = b.alloc_at(0x1000, 0x1000, PageFlags::READABLE).unwrap_err();
        assert!(matches!(err, Error::AlreadyAllocated { page: 1 }));
        // First call's pages still present
        assert_eq!(b.allocated_page_count(), 2);
    }

    #[test]
    fn alloc_at_rejects_zero_size() {
        let mut b = SparseBackend::new();
        assert_eq!(b.alloc_at(0x1000, 0, PageFlags::READABLE), Err(Error::ZeroSize));
    }

    #[test]
    fn alloc_at_rejects_overflow() {
        let mut b = SparseBackend::new();
        // Last page + one more would overflow u32+ address space
        let err = b
            .alloc_at(0xFFFF_F000, 0x2000, PageFlags::READABLE)
            .unwrap_err();
        assert!(matches!(err, Error::AddressOverflow { .. }));
    }

    #[test]
    fn dealloc_roundtrip() {
        let mut b = SparseBackend::new();
        b.alloc_at(0x0, 0x3000, PageFlags::READABLE).unwrap();
        assert_eq!(b.allocated_page_count(), 3);
        b.dealloc(0x0, 0x3000).unwrap();
        assert_eq!(b.allocated_page_count(), 0);
        let (alloc, flags) = b.page_table.get_addr_flags(0);
        assert!(!alloc);
        assert_eq!(flags.0, 0);
    }

    #[test]
    fn dealloc_rejects_unallocated() {
        let mut b = SparseBackend::new();
        assert!(matches!(
            b.dealloc(0x1000, 0x1000),
            Err(Error::NotAllocated { .. })
        ));
    }

    // -- read/write ------------------------------------------------

    #[test]
    fn write_then_read_single_byte() {
        let mut b = SparseBackend::new();
        b.alloc_at(0, 0x1000, PageFlags::READABLE | PageFlags::WRITABLE)
            .unwrap();
        b.write(0x42, &[0xAB, 0xCD]).unwrap();
        let mut out = [0u8; 2];
        b.read(0x42, &mut out).unwrap();
        assert_eq!(out, [0xAB, 0xCD]);
    }

    #[test]
    fn write_then_read_crossing_page_boundary() {
        let mut b = SparseBackend::new();
        b.alloc_at(0, 0x2000, PageFlags::READABLE | PageFlags::WRITABLE)
            .unwrap();
        // Write 8 bytes straddling 0xFFE..0x1006
        let data = [1u8, 2, 3, 4, 5, 6, 7, 8];
        b.write(0xFFE, &data).unwrap();
        let mut out = [0u8; 8];
        b.read(0xFFE, &mut out).unwrap();
        assert_eq!(out, data);
    }

    #[test]
    fn read_fails_without_readable_flag() {
        let mut b = SparseBackend::new();
        b.alloc_at(0, 0x1000, PageFlags::WRITABLE).unwrap();
        let mut out = [0u8; 1];
        assert!(matches!(
            b.read(0, &mut out),
            Err(Error::MissingFlags { .. })
        ));
    }

    #[test]
    fn write_fails_without_writable_flag() {
        let mut b = SparseBackend::new();
        b.alloc_at(0, 0x1000, PageFlags::READABLE).unwrap();
        assert!(matches!(
            b.write(0, &[0xFF]),
            Err(Error::MissingFlags { .. })
        ));
    }

    #[test]
    fn read_write_typed_u32_roundtrip() {
        let mut b = SparseBackend::new();
        b.alloc_at(0, 0x1000, PageFlags::READABLE | PageFlags::WRITABLE)
            .unwrap();
        b.write_le::<u32>(0x100, 0xDEAD_BEEF).unwrap();
        assert_eq!(b.read_le::<u32>(0x100).unwrap(), 0xDEAD_BEEF);
    }

    #[test]
    fn read_write_typed_u64_roundtrip() {
        let mut b = SparseBackend::new();
        b.alloc_at(0, 0x1000, PageFlags::READABLE | PageFlags::WRITABLE)
            .unwrap();
        b.write_le::<u64>(0x200, 0x1234_5678_9ABC_DEF0).unwrap();
        assert_eq!(b.read_le::<u64>(0x200).unwrap(), 0x1234_5678_9ABC_DEF0);
    }

    // -- page_protect ----------------------------------------------

    #[test]
    fn page_protect_adds_flags() {
        let mut b = SparseBackend::new();
        b.alloc_at(0, 0x1000, PageFlags::READABLE).unwrap();
        // Can't write yet
        assert!(b.write(0, &[0xFF]).is_err());
        // Add WRITABLE
        b.page_protect(0, 0x1000, PageFlags::WRITABLE, PageFlags::empty())
            .unwrap();
        // Now write succeeds
        b.write(0, &[0xFF]).unwrap();
    }

    #[test]
    fn page_protect_clears_flags() {
        let mut b = SparseBackend::new();
        b.alloc_at(0, 0x1000, PageFlags::READABLE | PageFlags::WRITABLE)
            .unwrap();
        b.page_protect(0, 0x1000, PageFlags::empty(), PageFlags::WRITABLE)
            .unwrap();
        assert!(b.write(0, &[0xFF]).is_err());
    }

    // -- ReservationTable ------------------------------------------

    #[test]
    fn reservation_fresh_is_zero() {
        let r = ReservationTable::new();
        assert_eq!(r.acquire(0), 0);
        assert_eq!(r.acquire(0x100), 0);
    }

    #[test]
    fn reservation_update_bumps_timestamp() {
        let r = ReservationTable::new();
        r.update(0);
        assert_eq!(r.acquire(0), 1);
        r.update(0);
        assert_eq!(r.acquire(0), 2);
    }

    #[test]
    fn reservation_rows_are_per_128_bytes() {
        let r = ReservationTable::new();
        r.update(0);
        // 0..127 all hit row 0
        assert_eq!(r.acquire(127), 1);
        // 128 starts row 1 — untouched
        assert_eq!(r.acquire(128), 0);
    }

    #[test]
    fn reservation_try_commit_succeeds_when_unchanged() {
        let r = ReservationTable::new();
        let prev = r.acquire(0);
        assert_eq!(r.try_commit(0, prev), Ok(prev + 1));
    }

    #[test]
    fn reservation_try_commit_fails_after_concurrent_update() {
        let r = ReservationTable::new();
        let prev = r.acquire(0);
        r.update(0); // simulates concurrent writer
        assert!(r.try_commit(0, prev).is_err());
    }
}
