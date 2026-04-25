//! Rust port of `rpcs3/Emu/Cell/lv2/sys_dbg.cpp` — PS3 LV2 debug syscalls
//! (read/write process memory, 131 lines C++).
//!
//! 2 entries:
//! * `sys_dbg_read_process_memory(pid, address, size, data)` — copies
//!   `size` bytes from guest `address` to caller buffer.
//! * `sys_dbg_write_process_memory(pid, address, size, data)` — writes
//!   `size` bytes from caller buffer to guest `address`. Has special
//!   path for the stack region (`address >> 28 == 0xD`) and for executable
//!   pages (must call `ppu_register_function_at` after the write).
//!
//! Validation cascade preserved EXATA do C++: pid != 1 → INVALID_ARGUMENTS,
//! null/zero size → INVALID_ARGUMENTS, unwritable destination → EFAULT,
//! unreadable source → EFAULT.
//!
//! The actual VM access is plugged via a `VmAccess` trait — production
//! wires it to the emulator memory map; tests use `MockVm`.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sys_dbg";

pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sys_dbg_read_process_memory",
    "sys_dbg_write_process_memory",
];

/// Error from `Modules/sys_lv2dbg.h` byte-exato (only INVALIDARGUMENTS
/// is referenced by sys_dbg.cpp).
pub const CELL_LV2DBG_ERROR_DEINVALIDARGUMENTS: CellError = CellError(0x8001_0402);
pub const CELL_EFAULT: CellError = CellError(0x8001_000D);

/// Mirror of vm::page_* flags (only the ones referenced by this module).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageFlags(pub u32);
impl PageFlags {
    pub const READABLE: PageFlags = PageFlags(0x01);
    pub const WRITABLE: PageFlags = PageFlags(0x02);
    pub const EXECUTABLE: PageFlags = PageFlags(0x04);
}

/// Pluggable VM access — production wires to vm::base/check_addr; tests
/// use `MockVm`.
pub trait VmAccess {
    /// Mirror of `vm::check_addr(addr, flags, size)`.
    fn check_addr(&self, addr: u32, flags: PageFlags, size: u32) -> bool;
    /// Mirror of `std::memmove(vm::base(addr), data, size)`.
    fn memmove_into_guest(&mut self, addr: u32, data: &[u8]);
    /// Mirror of `std::memmove(data, vm::base(addr), size)` — read FROM guest.
    fn memmove_from_guest(&self, addr: u32, out: &mut [u8]);
    /// Called after writing to executable pages so the JIT/interpreter
    /// re-decodes the affected range. Mirror of `ppu_register_function_at`.
    fn register_function_at(&mut self, addr: u32, size: u32);
}

/// Minimal in-memory VM for tests.
#[derive(Debug)]
pub struct MockVm {
    /// Page table: address (4K-aligned) → flags.
    pub pages: Vec<(u32, PageFlags)>,
    /// Backing store: address → byte (sparse).
    pub bytes: Vec<(u32, u8)>,
    /// Trace of `register_function_at` calls.
    pub registered_ranges: Vec<(u32, u32)>,
    pub page_size: u32,
}

impl Default for MockVm {
    fn default() -> Self {
        Self {
            pages: Vec::new(),
            bytes: Vec::new(),
            registered_ranges: Vec::new(),
            page_size: 0x1000, // 4 KiB default
        }
    }
}

impl MockVm {
    pub fn new() -> Self {
        Self::default()
    }

    /// Maps a contiguous region with given flags.
    pub fn map(&mut self, addr: u32, size: u32, flags: PageFlags) {
        let end = addr.saturating_add(size);
        let mut a = addr & !(self.page_size - 1);
        while a < end {
            self.pages.push((a, flags));
            a = a.saturating_add(self.page_size);
        }
    }

    /// Helper: read sparse-byte storage (returns 0 for unmapped bytes).
    fn read_byte(&self, addr: u32) -> u8 {
        self.bytes
            .iter()
            .find(|(a, _)| *a == addr)
            .map(|(_, b)| *b)
            .unwrap_or(0)
    }

    /// Helper: upsert sparse byte.
    fn write_byte(&mut self, addr: u32, value: u8) {
        if let Some(slot) = self.bytes.iter_mut().find(|(a, _)| *a == addr) {
            slot.1 = value;
        } else {
            self.bytes.push((addr, value));
        }
    }
}

impl VmAccess for MockVm {
    fn check_addr(&self, addr: u32, flags: PageFlags, size: u32) -> bool {
        let end = addr.saturating_add(size);
        let mut a = addr & !(self.page_size - 1);
        while a < end {
            let page_flags = match self.pages.iter().find(|(p, _)| *p == a) {
                Some((_, f)) => *f,
                None => return false,
            };
            if (page_flags.0 & flags.0) != flags.0 {
                return false;
            }
            a = a.saturating_add(self.page_size);
        }
        true
    }

    fn memmove_into_guest(&mut self, addr: u32, data: &[u8]) {
        for (i, b) in data.iter().enumerate() {
            self.write_byte(addr + i as u32, *b);
        }
    }

    fn memmove_from_guest(&self, addr: u32, out: &mut [u8]) {
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = self.read_byte(addr + i as u32);
        }
    }

    fn register_function_at(&mut self, addr: u32, size: u32) {
        self.registered_ranges.push((addr, size));
    }
}

#[derive(Debug, Default)]
pub struct SysDbg {
    pub read_calls: u64,
    pub write_calls: u64,
}

impl SysDbg {
    pub fn new() -> Self {
        Self::default()
    }

    /// `sys_dbg_read_process_memory(pid, address, size, data)` — cpp:16-48.
    /// Validation order preserved EXATA: pid → size/data → write-check
    /// destination → read-check source → memmove.
    pub fn read_process_memory<V: VmAccess>(
        &mut self,
        vm: &V,
        pid: i32,
        address: u32,
        size: u32,
        // The "data" out-param is split: data_addr is the guest-side address of
        // the destination buffer (used for write-check), out is the actual host
        // slice to fill.
        data_addr: u32,
        out: Option<&mut [u8]>,
    ) -> Result<(), CellError> {
        self.read_calls = self.read_calls.saturating_add(1);
        if pid != 1 {
            return Err(CELL_LV2DBG_ERROR_DEINVALIDARGUMENTS);
        }
        if size == 0 || out.is_none() {
            return Err(CELL_LV2DBG_ERROR_DEINVALIDARGUMENTS);
        }
        if !vm.check_addr(data_addr, PageFlags::WRITABLE, size) {
            return Err(CELL_EFAULT);
        }
        if !vm.check_addr(address, PageFlags::READABLE, size) {
            return Err(CELL_EFAULT);
        }
        let out = out.unwrap();
        vm.memmove_from_guest(address, &mut out[..size as usize]);
        Ok(())
    }

    /// `sys_dbg_write_process_memory(pid, address, size, data)` — cpp:50-131.
    /// Has special path for stack pages (`address >> 28 == 0xD`) which use
    /// raw memmove only, and for executable pages which require
    /// `register_function_at` after the write.
    pub fn write_process_memory<V: VmAccess>(
        &mut self,
        vm: &mut V,
        pid: i32,
        address: u32,
        size: u32,
        data_addr: u32,
        data: Option<&[u8]>,
    ) -> Result<(), CellError> {
        self.write_calls = self.write_calls.saturating_add(1);
        if pid != 1 {
            return Err(CELL_LV2DBG_ERROR_DEINVALIDARGUMENTS);
        }
        if size == 0 || data.is_none() {
            return Err(CELL_LV2DBG_ERROR_DEINVALIDARGUMENTS);
        }
        if !vm.check_addr(data_addr, PageFlags::READABLE, size) {
            return Err(CELL_EFAULT);
        }
        if !vm.check_addr(address, PageFlags::READABLE, size) {
            return Err(CELL_EFAULT);
        }
        let data = data.unwrap();

        // Stack region fast-path cpp:87-92.
        if (address >> 28) == 0xD {
            vm.memmove_into_guest(address, &data[..size as usize]);
            return Ok(());
        }

        // General path cpp:94-128 — segment by executable boundaries
        // (page-aligned to 0x10000 = 64 KiB) and call register_function_at
        // after each contiguous executable run.
        let end = address.checked_add(size).ok_or(CELL_EFAULT)?;
        let mut i = address;
        let mut exec_update_size = 0u32;

        while i < end {
            let next_boundary = (i & !(0x10000 - 1)).saturating_add(0x10000);
            let op_size = next_boundary.min(end) - i;

            let is_exec = vm.check_addr(
                i,
                PageFlags(PageFlags::EXECUTABLE.0 | PageFlags::READABLE.0),
                1,
            );

            if is_exec {
                exec_update_size += op_size;
                i += op_size;
            }

            if (!is_exec || i >= end) && exec_update_size > 0 {
                let before_addr = i - exec_update_size;
                let offset = (before_addr - address) as usize;
                vm.memmove_into_guest(
                    before_addr,
                    &data[offset..offset + exec_update_size as usize],
                );
                vm.register_function_at(before_addr, exec_update_size);
                exec_update_size = 0;
                if i >= end {
                    break;
                }
            }

            if !is_exec {
                let offset = (i - address) as usize;
                vm.memmove_into_guest(i, &data[offset..offset + op_size as usize]);
                i += op_size;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entries() {
        assert_eq!(MODULE_NAME, "sys_dbg");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 2);
    }

    #[test]
    fn errors_byte_exact() {
        assert_eq!(CELL_LV2DBG_ERROR_DEINVALIDARGUMENTS.0, 0x8001_0402);
        assert_eq!(CELL_EFAULT.0, 0x8001_000D);
    }

    #[test]
    fn read_pid_not_1_invalid() {
        let mut dbg = SysDbg::new();
        let vm = MockVm::new();
        let mut buf = [0u8; 16];
        assert_eq!(
            dbg.read_process_memory(&vm, 0, 0x1000, 16, 0x2000, Some(&mut buf)),
            Err(CELL_LV2DBG_ERROR_DEINVALIDARGUMENTS)
        );
    }

    #[test]
    fn read_zero_size_invalid() {
        let mut dbg = SysDbg::new();
        let vm = MockVm::new();
        let mut buf = [0u8; 16];
        assert_eq!(
            dbg.read_process_memory(&vm, 1, 0x1000, 0, 0x2000, Some(&mut buf)),
            Err(CELL_LV2DBG_ERROR_DEINVALIDARGUMENTS)
        );
    }

    #[test]
    fn read_null_data_invalid() {
        let mut dbg = SysDbg::new();
        let vm = MockVm::new();
        assert_eq!(
            dbg.read_process_memory(&vm, 1, 0x1000, 16, 0x2000, None),
            Err(CELL_LV2DBG_ERROR_DEINVALIDARGUMENTS)
        );
    }

    #[test]
    fn read_unwritable_dest_efault() {
        let mut dbg = SysDbg::new();
        let mut vm = MockVm::new();
        // Source mapped readable but destination NOT writable.
        vm.map(0x1000, 0x1000, PageFlags::READABLE);
        let mut buf = [0u8; 16];
        assert_eq!(
            dbg.read_process_memory(&vm, 1, 0x1000, 16, 0x2000, Some(&mut buf)),
            Err(CELL_EFAULT)
        );
    }

    #[test]
    fn read_happy_path_copies_bytes() {
        let mut dbg = SysDbg::new();
        let mut vm = MockVm::new();
        vm.map(0x1000, 0x1000, PageFlags::READABLE);
        vm.map(0x2000, 0x1000, PageFlags::WRITABLE);
        for i in 0..8 {
            vm.write_byte(0x1000 + i, (i + 0xA) as u8);
        }
        let mut buf = [0u8; 8];
        dbg.read_process_memory(&vm, 1, 0x1000, 8, 0x2000, Some(&mut buf)).unwrap();
        assert_eq!(buf, [0xA, 0xB, 0xC, 0xD, 0xE, 0xF, 0x10, 0x11]);
    }

    #[test]
    fn write_pid_not_1_invalid() {
        let mut dbg = SysDbg::new();
        let mut vm = MockVm::new();
        let data = [0u8; 8];
        assert_eq!(
            dbg.write_process_memory(&mut vm, 99, 0x1000, 8, 0x2000, Some(&data)),
            Err(CELL_LV2DBG_ERROR_DEINVALIDARGUMENTS)
        );
    }

    #[test]
    fn write_stack_fast_path() {
        let mut dbg = SysDbg::new();
        let mut vm = MockVm::new();
        // Stack region: address >> 28 == 0xD, so 0xD000_0000.
        vm.map(0xD000_0000, 0x1000, PageFlags::READABLE);
        vm.map(0x2000, 0x1000, PageFlags::READABLE);
        let data = [0xAA, 0xBB, 0xCC, 0xDD];
        dbg.write_process_memory(&mut vm, 1, 0xD000_0000, 4, 0x2000, Some(&data)).unwrap();
        assert_eq!(vm.read_byte(0xD000_0000), 0xAA);
        assert_eq!(vm.read_byte(0xD000_0001), 0xBB);
        assert_eq!(vm.read_byte(0xD000_0002), 0xCC);
        assert_eq!(vm.read_byte(0xD000_0003), 0xDD);
        // Stack path doesn't call register_function_at.
        assert!(vm.registered_ranges.is_empty());
    }

    #[test]
    fn write_executable_path_calls_register_function_at() {
        let mut dbg = SysDbg::new();
        let mut vm = MockVm::new();
        // Map source as readable + dest as exec+readable.
        vm.map(0x2000, 0x1000, PageFlags::READABLE);
        vm.map(
            0x1_0000,
            0x1000,
            PageFlags(PageFlags::EXECUTABLE.0 | PageFlags::READABLE.0),
        );
        let data = [0x90, 0x90, 0x90, 0x90];
        dbg.write_process_memory(&mut vm, 1, 0x1_0000, 4, 0x2000, Some(&data)).unwrap();
        // Should have registered the patched range.
        assert!(!vm.registered_ranges.is_empty());
        assert_eq!(vm.registered_ranges[0].0, 0x1_0000);
        assert_eq!(vm.registered_ranges[0].1, 4);
        // Bytes copied.
        assert_eq!(vm.read_byte(0x1_0000), 0x90);
    }

    #[test]
    fn write_data_normal_path_no_register_call() {
        let mut dbg = SysDbg::new();
        let mut vm = MockVm::new();
        // Normal writable+readable region (not executable, not 0xD-prefix).
        vm.map(0x4000_0000, 0x1000, PageFlags::READABLE);
        vm.map(0x5000_0000, 0x1000, PageFlags::READABLE);
        let data = [0x42; 16];
        dbg.write_process_memory(
            &mut vm,
            1,
            0x4000_0000,
            16,
            0x5000_0000,
            Some(&data),
        )
        .unwrap();
        assert_eq!(vm.read_byte(0x4000_0000), 0x42);
        assert_eq!(vm.read_byte(0x4000_000F), 0x42);
        // Not executable → no register_function_at calls.
        assert!(vm.registered_ranges.is_empty());
    }

    #[test]
    fn counters_track_calls() {
        let mut dbg = SysDbg::new();
        let mut vm = MockVm::new();
        let _ = dbg.read_process_memory(&vm, 1, 0, 0, 0, None);
        let _ = dbg.read_process_memory(&vm, 1, 0, 0, 0, None);
        let _ = dbg.write_process_memory(&mut vm, 1, 0, 0, 0, None);
        assert_eq!(dbg.read_calls, 2);
        assert_eq!(dbg.write_calls, 1);
    }
}
