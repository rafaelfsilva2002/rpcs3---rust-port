//! `rpcs3-memory` — PS3 memory layout types and constants.
//!
//! **Wave 4a scope**: safe types only. No raw pointer access, no
//! allocator, no reservation table ops. That intentionally defers the
//! hard part (shared mutable memory with the JIT) to Wave 4b, where
//! it lives inside a narrow `unsafe` wall with `miri`/fuzz coverage.
//!
//! This crate freezes:
//! * Page-flag bit values (matching `vm::page_info_t` in `vm.h:57-69`).
//! * Memory region enum (`vm::memory_location_t` in `vm.h:43-55`).
//! * Address-space constants (PS3 RAM = 4 GB; page = 4 KB; reservation
//!   block = 128 bytes).
//! * Address windows used by `ppu_load_exec` and `ppu_load_overlay`.
//! * Block flags (page size 4K/64K/1M, stack_guarded, preallocated).
//!
//! Anchors for the C++ source are included inline so regressions are
//! easy to trace.

// =====================================================================
// Address-space constants
// =====================================================================

/// PS3 guest address space is 32-bit — 4 GB total.
pub const ADDRESS_SPACE_SIZE: u64 = 0x1_0000_0000;

/// Page granularity used by `vm::g_pages[]` in `vm.h:86`.
pub const PAGE_SIZE: u32 = 4096;

/// Page count covering the full PS3 address space.
/// Matches `0x100000000 / 4096 = 1_048_576` (see vm.h:86).
pub const PAGE_COUNT: u32 = (ADDRESS_SPACE_SIZE / PAGE_SIZE as u64) as u32;

/// Reservation block granularity (128 bytes). PS3 LL/SC reserves
/// cache-line-aligned 128-byte blocks.
pub const RESERVATION_BLOCK_SIZE: u32 = 128;

/// Reservation table row count: 65536 / 128 = 512.
/// Matches the `g_reservations[65536 / 128 * 64]` declaration in vm.h:37.
pub const RESERVATION_ROWS: u32 = 65536 / RESERVATION_BLOCK_SIZE;

/// Bytes per reservation-table entry (includes the 64-bit timestamp
/// plus waiter tracking state).
pub const RESERVATION_ENTRY_BYTES: u32 = 64;

// =====================================================================
// Address windows (PPU load validation)
// =====================================================================

/// Main RAM window for `ppu_load_exec`: [0x00000000, 0x30000000).
/// See `PPUModule.cpp:2080-2120` for the bounds check.
pub const PPU_MAIN_RAM_MIN: u32 = 0x0000_0000;
pub const PPU_MAIN_RAM_MAX: u32 = 0x3000_0000;

/// Overlay window for `ppu_load_overlay`: [0x30000000, 0x40000000).
pub const OVERLAY_MIN: u32 = 0x3000_0000;
pub const OVERLAY_MAX: u32 = 0x4000_0000;

/// 2 GB boundary where the SPU address region begins in the unified
/// address space. Matches the `g_exec_addr_seg_offset = 0x2'0000'0000`
/// constant in vm.h:39 (used for exec address segmentation).
pub const EXEC_ADDR_SEG_OFFSET: u64 = 0x2_0000_0000;

// =====================================================================
// Page flags — vm::page_info_t (vm.h:57-69)
// =====================================================================

/// Bit flags attached to each 4 KB page.
/// **Exact mirror** of `vm::page_info_t`; any bit change silently breaks
/// `check_addr()` and the reservation system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct PageFlags(pub u8);

impl PageFlags {
    pub const READABLE: Self = Self(1 << 0);
    pub const WRITABLE: Self = Self(1 << 1);
    pub const EXECUTABLE: Self = Self(1 << 2);
    pub const FAULT_NOTIFICATION: Self = Self(1 << 3);
    pub const NO_RESERVATIONS: Self = Self(1 << 4);
    pub const PAGE_64K_SIZE: Self = Self(1 << 5);
    pub const PAGE_1M_SIZE: Self = Self(1 << 6);
    pub const ALLOCATED: Self = Self(1 << 7);

    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[must_use]
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    #[must_use]
    pub const fn complement(self) -> Self {
        Self(!self.0)
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub const fn is_allocated(self) -> bool {
        self.contains(Self::ALLOCATED)
    }

    #[must_use]
    pub const fn is_readable(self) -> bool {
        self.contains(Self::READABLE)
    }

    #[must_use]
    pub const fn is_writable(self) -> bool {
        self.contains(Self::WRITABLE)
    }

    #[must_use]
    pub const fn is_executable(self) -> bool {
        self.contains(Self::EXECUTABLE)
    }
}

impl core::ops::BitOr for PageFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl core::ops::BitAnd for PageFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        self.intersection(rhs)
    }
}

// =====================================================================
// Memory regions — vm::memory_location_t (vm.h:43-55)
// =====================================================================

/// Logical memory regions used by `vm::alloc(size, location, ...)`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MemoryLocation {
    Main = 0,
    User64k = 1,
    User1m = 2,
    RsxContext = 3,
    Video = 4,
    Stack = 5,
    Spu = 6,
}

/// Sentinel used by C++ to mean "any location".
pub const MEMORY_LOCATION_ANY: u32 = 0xFFFF_FFFF;

/// Total number of distinct regions (excluding the ANY sentinel).
pub const MEMORY_LOCATION_MAX: u32 = 7;

// =====================================================================
// Block flags — vm::block_flags_3 (vm.h:122-130)
// =====================================================================

/// Flags passed to `vm::falloc` / block allocator to describe the
/// requested page shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct BlockFlags(pub u32);

impl BlockFlags {
    pub const PAGE_SIZE_4K: Self = Self(0x100);
    pub const PAGE_SIZE_64K: Self = Self(0x200);
    pub const PAGE_SIZE_1M: Self = Self(0x400);
    pub const PAGE_SIZE_MASK: Self = Self(0xF00);
    pub const STACK_GUARDED: Self = Self(0x10);
    pub const PREALLOCATED: Self = Self(0x20);

    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[must_use]
    pub const fn page_size(self) -> Option<u32> {
        match Self(self.0 & Self::PAGE_SIZE_MASK.0) {
            Self::PAGE_SIZE_4K => Some(4 * 1024),
            Self::PAGE_SIZE_64K => Some(64 * 1024),
            Self::PAGE_SIZE_1M => Some(1024 * 1024),
            _ => None,
        }
    }
}

// =====================================================================
// Compile-time assertions: these values are load-bearing contracts.
// =====================================================================

const _: () = {
    assert!(PAGE_SIZE == 4096);
    assert!(PAGE_COUNT == 1_048_576);
    assert!(RESERVATION_BLOCK_SIZE == 128);
    assert!(RESERVATION_ROWS == 512);
    assert!(ADDRESS_SPACE_SIZE == 0x1_0000_0000);

    assert!(PageFlags::READABLE.0 == 0x01);
    assert!(PageFlags::WRITABLE.0 == 0x02);
    assert!(PageFlags::EXECUTABLE.0 == 0x04);
    assert!(PageFlags::FAULT_NOTIFICATION.0 == 0x08);
    assert!(PageFlags::NO_RESERVATIONS.0 == 0x10);
    assert!(PageFlags::PAGE_64K_SIZE.0 == 0x20);
    assert!(PageFlags::PAGE_1M_SIZE.0 == 0x40);
    assert!(PageFlags::ALLOCATED.0 == 0x80);

    assert!(MemoryLocation::Main as u32 == 0);
    assert!(MemoryLocation::Stack as u32 == 5);
    assert!(MemoryLocation::Spu as u32 == 6);

    assert!(BlockFlags::PAGE_SIZE_4K.0 == 0x100);
    assert!(BlockFlags::PAGE_SIZE_64K.0 == 0x200);
    assert!(BlockFlags::PAGE_SIZE_1M.0 == 0x400);
    assert!(BlockFlags::STACK_GUARDED.0 == 0x10);
};

// =====================================================================
// Simple utilities around the type constants
// =====================================================================

/// Convert an address to its 4 KB page index. Equivalent to
/// `addr / PAGE_SIZE` — exposed to keep call sites explicit about
/// the unit conversion.
#[must_use]
pub const fn address_to_page(addr: u32) -> u32 {
    addr / PAGE_SIZE
}

/// Convert an address to its 128-byte reservation block index.
/// Used by the reservation table (`vm::g_reservations`).
#[must_use]
pub const fn address_to_reservation_row(addr: u32) -> u32 {
    (addr & 0xFFFF) / RESERVATION_BLOCK_SIZE
}

/// True if `addr..(addr+size)` fits inside the PPU main RAM window.
#[must_use]
pub const fn is_in_ppu_main_ram(addr: u32, size: u32) -> bool {
    let Some(end) = addr.checked_add(size) else {
        return false;
    };
    addr >= PPU_MAIN_RAM_MIN && end <= PPU_MAIN_RAM_MAX
}

/// True if `addr..(addr+size)` fits inside the PPU overlay window.
#[must_use]
pub const fn is_in_overlay(addr: u32, size: u32) -> bool {
    let Some(end) = addr.checked_add(size) else {
        return false;
    };
    addr >= OVERLAY_MIN && end <= OVERLAY_MAX
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- Constants -------------------------------------------------

    #[test]
    fn address_space_and_page_count_frozen() {
        assert_eq!(PAGE_SIZE, 4096);
        assert_eq!(PAGE_COUNT, 0x10_0000);
        assert_eq!(ADDRESS_SPACE_SIZE, 0x1_0000_0000);
    }

    #[test]
    fn reservation_table_shape_frozen() {
        assert_eq!(RESERVATION_BLOCK_SIZE, 128);
        assert_eq!(RESERVATION_ROWS, 512);
        assert_eq!(RESERVATION_ENTRY_BYTES, 64);
        // Matches g_reservations[65536 / 128 * 64] in vm.h:37.
        assert_eq!(RESERVATION_ROWS * RESERVATION_ENTRY_BYTES, 32768);
    }

    #[test]
    fn ppu_main_ram_window_anchors() {
        assert_eq!(PPU_MAIN_RAM_MIN, 0x0000_0000);
        assert_eq!(PPU_MAIN_RAM_MAX, 0x3000_0000);
        assert_eq!(OVERLAY_MIN, 0x3000_0000);
        assert_eq!(OVERLAY_MAX, 0x4000_0000);
    }

    // -- PageFlags -------------------------------------------------

    #[test]
    fn page_flag_bit_values_frozen() {
        assert_eq!(PageFlags::READABLE.0, 0x01);
        assert_eq!(PageFlags::WRITABLE.0, 0x02);
        assert_eq!(PageFlags::EXECUTABLE.0, 0x04);
        assert_eq!(PageFlags::FAULT_NOTIFICATION.0, 0x08);
        assert_eq!(PageFlags::NO_RESERVATIONS.0, 0x10);
        assert_eq!(PageFlags::PAGE_64K_SIZE.0, 0x20);
        assert_eq!(PageFlags::PAGE_1M_SIZE.0, 0x40);
        assert_eq!(PageFlags::ALLOCATED.0, 0x80);
    }

    #[test]
    fn page_flags_union_and_contains() {
        let f = PageFlags::READABLE | PageFlags::WRITABLE | PageFlags::ALLOCATED;
        assert!(f.is_readable());
        assert!(f.is_writable());
        assert!(f.is_allocated());
        assert!(!f.is_executable());
        assert!(f.contains(PageFlags::READABLE));
        assert!(f.contains(PageFlags::WRITABLE | PageFlags::ALLOCATED));
        assert!(!f.contains(PageFlags::EXECUTABLE));
    }

    #[test]
    fn page_flags_empty() {
        let f = PageFlags::empty();
        assert!(f.is_empty());
        assert!(!f.is_allocated());
    }

    // -- MemoryLocation --------------------------------------------

    #[test]
    fn memory_location_ordinals_frozen() {
        assert_eq!(MemoryLocation::Main as u32, 0);
        assert_eq!(MemoryLocation::User64k as u32, 1);
        assert_eq!(MemoryLocation::User1m as u32, 2);
        assert_eq!(MemoryLocation::RsxContext as u32, 3);
        assert_eq!(MemoryLocation::Video as u32, 4);
        assert_eq!(MemoryLocation::Stack as u32, 5);
        assert_eq!(MemoryLocation::Spu as u32, 6);
    }

    // -- BlockFlags ------------------------------------------------

    #[test]
    fn block_flags_page_size_lookup() {
        assert_eq!(BlockFlags::PAGE_SIZE_4K.page_size(), Some(4096));
        assert_eq!(BlockFlags::PAGE_SIZE_64K.page_size(), Some(65536));
        assert_eq!(BlockFlags::PAGE_SIZE_1M.page_size(), Some(1024 * 1024));
        assert_eq!(BlockFlags::empty().page_size(), None);
    }

    #[test]
    fn block_flags_mask_is_stable() {
        assert_eq!(BlockFlags::PAGE_SIZE_MASK.0, 0xF00);
        assert!(
            BlockFlags::PAGE_SIZE_4K.0 & BlockFlags::PAGE_SIZE_MASK.0
                == BlockFlags::PAGE_SIZE_4K.0
        );
    }

    // -- Address utility fns ---------------------------------------

    #[test]
    fn address_to_page_examples() {
        assert_eq!(address_to_page(0), 0);
        assert_eq!(address_to_page(0xFFF), 0);
        assert_eq!(address_to_page(0x1000), 1);
        assert_eq!(address_to_page(0x2000), 2);
        assert_eq!(address_to_page(0x1000_0000), 0x1_0000);
    }

    #[test]
    fn address_to_reservation_row_examples() {
        assert_eq!(address_to_reservation_row(0), 0);
        assert_eq!(address_to_reservation_row(127), 0);
        assert_eq!(address_to_reservation_row(128), 1);
        assert_eq!(address_to_reservation_row(256), 2);
        // Wraps at the 64 KB reservation window.
        assert_eq!(address_to_reservation_row(0x1_0000), 0);
    }

    #[test]
    fn is_in_ppu_main_ram_bounds() {
        assert!(is_in_ppu_main_ram(0, 1));
        assert!(is_in_ppu_main_ram(0x1_0000, 0x1000));
        assert!(is_in_ppu_main_ram(0x2FFF_FFFF, 1));
        assert!(is_in_ppu_main_ram(0, 0x3000_0000)); // exactly fills
        assert!(!is_in_ppu_main_ram(0x3000_0000, 1)); // starts at overlay
        assert!(!is_in_ppu_main_ram(0x2FFF_F000, 0x2000)); // crosses boundary
    }

    #[test]
    fn is_in_overlay_bounds() {
        assert!(is_in_overlay(0x3000_0000, 1));
        assert!(is_in_overlay(0x3FFF_F000, 0x1000));
        assert!(!is_in_overlay(0x4000_0000, 1));
        assert!(!is_in_overlay(0, 1));
    }

    #[test]
    fn is_in_range_handles_overflow() {
        // addr + size overflowing u32 should be rejected.
        assert!(!is_in_ppu_main_ram(u32::MAX, 1));
        assert!(!is_in_overlay(u32::MAX, 1));
    }
}
