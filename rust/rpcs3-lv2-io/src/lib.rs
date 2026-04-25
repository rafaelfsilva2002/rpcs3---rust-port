//! Rust port of `rpcs3/Emu/Cell/lv2/sys_io.cpp` — PS3 LV2 I/O buffer
//! syscalls (4 entries, 75 lines C++).
//!
//! `sys_io_buffer_create/destroy/allocate/free` — generic kernel-side I/O
//! buffer pool used by various LV2 subsystems. Models idm allocation +
//! per-buffer per-block allocation tracking.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sys_io";

pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sys_io_buffer_create",
    "sys_io_buffer_destroy",
    "sys_io_buffer_allocate",
    "sys_io_buffer_free",
];

pub const CELL_EFAULT: CellError = CellError(0x8001_000D);
pub const CELL_ESRCH: CellError = CellError(0x8001_0005);
pub const CELL_ENOMEM: CellError = CellError(0x8001_0004);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IoBuffer {
    pub handle: u32,
    pub block_count: u32,
    pub block_size: u32,
    pub blocks: u32,
    pub unk1: u32,
}

#[derive(Debug, Default)]
pub struct SysIo {
    pub buffers: Vec<IoBuffer>,
    /// (buffer_handle, allocated_block_addr) records
    pub allocations: Vec<(u32, u32)>,
    pub next_handle: u32,
    pub next_block_addr: u32,

    pub create_calls: u64,
    pub destroy_calls: u64,
    pub allocate_calls: u64,
    pub free_calls: u64,
}

impl SysIo {
    pub fn new() -> Self {
        Self {
            next_handle: 1,
            next_block_addr: 0x4000_0000,
            ..Default::default()
        }
    }

    /// `sys_io_buffer_create(block_count, block_size, blocks, unk1, handle)`.
    pub fn buffer_create(
        &mut self,
        block_count: u32,
        block_size: u32,
        blocks: u32,
        unk1: u32,
        handle_out: Option<&mut u32>,
    ) -> Result<(), CellError> {
        self.create_calls = self.create_calls.saturating_add(1);
        let slot = handle_out.ok_or(CELL_EFAULT)?;
        let h = self.next_handle;
        self.next_handle = self.next_handle.wrapping_add(1);
        self.buffers.push(IoBuffer {
            handle: h,
            block_count,
            block_size,
            blocks,
            unk1,
        });
        *slot = h;
        Ok(())
    }

    /// `sys_io_buffer_destroy(handle)` — upstream silently no-ops on unknown.
    pub fn buffer_destroy(&mut self, handle: u32) -> Result<(), CellError> {
        self.destroy_calls = self.destroy_calls.saturating_add(1);
        self.buffers.retain(|b| b.handle != handle);
        // Also drain any orphaned allocations.
        self.allocations.retain(|(h, _)| *h != handle);
        Ok(())
    }

    /// `sys_io_buffer_allocate(handle, block)`.
    pub fn buffer_allocate(
        &mut self,
        handle: u32,
        block_out: Option<&mut u32>,
    ) -> Result<(), CellError> {
        self.allocate_calls = self.allocate_calls.saturating_add(1);
        let slot = block_out.ok_or(CELL_EFAULT)?;
        let buf = self
            .buffers
            .iter()
            .find(|b| b.handle == handle)
            .copied()
            .ok_or(CELL_ESRCH)?;
        let size = buf
            .block_count
            .checked_mul(buf.block_size)
            .ok_or(CELL_ENOMEM)?;
        let addr = self.next_block_addr;
        self.next_block_addr = self
            .next_block_addr
            .checked_add(size)
            .ok_or(CELL_ENOMEM)?;
        self.allocations.push((handle, addr));
        *slot = addr;
        Ok(())
    }

    /// `sys_io_buffer_free(handle, block)`.
    pub fn buffer_free(&mut self, handle: u32, block: u32) -> Result<(), CellError> {
        self.free_calls = self.free_calls.saturating_add(1);
        if !self.buffers.iter().any(|b| b.handle == handle) {
            return Err(CELL_ESRCH);
        }
        self.allocations.retain(|(h, b)| !(*h == handle && *b == block));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entries() {
        assert_eq!(MODULE_NAME, "sys_io");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 4);
    }

    #[test]
    fn create_null_handle_efault() {
        let mut m = SysIo::new();
        assert_eq!(m.buffer_create(4, 0x1000, 8, 0, None), Err(CELL_EFAULT));
    }

    #[test]
    fn create_allocates_handle() {
        let mut m = SysIo::new();
        let mut h = 0u32;
        m.buffer_create(4, 0x1000, 8, 0, Some(&mut h)).unwrap();
        assert_eq!(h, 1);
        let mut h2 = 0u32;
        m.buffer_create(2, 0x100, 4, 0, Some(&mut h2)).unwrap();
        assert_eq!(h2, 2);
        assert_eq!(m.buffers.len(), 2);
    }

    #[test]
    fn allocate_unknown_handle_esrch() {
        let mut m = SysIo::new();
        let mut b = 0u32;
        assert_eq!(m.buffer_allocate(99, Some(&mut b)), Err(CELL_ESRCH));
    }

    #[test]
    fn allocate_writes_block_addr() {
        let mut m = SysIo::new();
        let mut h = 0u32;
        m.buffer_create(4, 0x1000, 8, 0, Some(&mut h)).unwrap();
        let mut b = 0u32;
        m.buffer_allocate(h, Some(&mut b)).unwrap();
        assert_eq!(b, 0x4000_0000);
        // 4 blocks × 0x1000 = 0x4000 size.
        let mut b2 = 0u32;
        m.buffer_allocate(h, Some(&mut b2)).unwrap();
        assert_eq!(b2, 0x4000_4000);
    }

    #[test]
    fn allocate_null_block_efault() {
        let mut m = SysIo::new();
        let mut h = 0u32;
        m.buffer_create(4, 0x1000, 8, 0, Some(&mut h)).unwrap();
        assert_eq!(m.buffer_allocate(h, None), Err(CELL_EFAULT));
    }

    #[test]
    fn free_unknown_buffer_esrch() {
        let mut m = SysIo::new();
        assert_eq!(m.buffer_free(99, 0x1000), Err(CELL_ESRCH));
    }

    #[test]
    fn free_removes_allocation() {
        let mut m = SysIo::new();
        let mut h = 0u32;
        m.buffer_create(1, 0x100, 1, 0, Some(&mut h)).unwrap();
        let mut b = 0u32;
        m.buffer_allocate(h, Some(&mut b)).unwrap();
        assert_eq!(m.allocations.len(), 1);
        m.buffer_free(h, b).unwrap();
        assert!(m.allocations.is_empty());
    }

    #[test]
    fn destroy_drains_orphan_allocations() {
        let mut m = SysIo::new();
        let mut h = 0u32;
        m.buffer_create(1, 0x100, 1, 0, Some(&mut h)).unwrap();
        let mut b = 0u32;
        m.buffer_allocate(h, Some(&mut b)).unwrap();
        m.buffer_destroy(h).unwrap();
        assert!(m.buffers.is_empty());
        assert!(m.allocations.is_empty());
    }

    #[test]
    fn destroy_unknown_is_noop_ok() {
        let mut m = SysIo::new();
        m.buffer_destroy(99).unwrap();
    }

    #[test]
    fn full_io_buffer_lifecycle_smoke() {
        let mut m = SysIo::new();
        let mut h1 = 0u32;
        let mut h2 = 0u32;
        m.buffer_create(2, 0x800, 4, 0, Some(&mut h1)).unwrap();
        m.buffer_create(8, 0x100, 16, 0, Some(&mut h2)).unwrap();

        let mut b1 = 0u32;
        let mut b2 = 0u32;
        m.buffer_allocate(h1, Some(&mut b1)).unwrap();
        m.buffer_allocate(h2, Some(&mut b2)).unwrap();
        assert_eq!(m.allocations.len(), 2);

        m.buffer_free(h1, b1).unwrap();
        m.buffer_destroy(h2).unwrap();
        assert_eq!(m.buffers.len(), 1);
        assert!(m.allocations.is_empty());

        m.buffer_destroy(h1).unwrap();
        assert!(m.buffers.is_empty());
    }
}
