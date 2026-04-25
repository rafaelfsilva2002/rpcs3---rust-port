//! Rust port of `rpcs3/Emu/Cell/lv2/sys_trace.cpp` — PS3 LV2 trace syscalls
//! (10 entries, 70 lines C++).
//!
//! All entries return `CELL_ENOSYS=0x80010001` byte-exato — these syscalls
//! are DEX/DECR-only (development consoles), so retail returns "not
//! supported" universally. Upstream cpp:10 has TODO comment about DEX/DECR
//! mode support.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sys_trace";

pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sys_trace_create",
    "sys_trace_start",
    "sys_trace_stop",
    "sys_trace_update_top_index",
    "sys_trace_destroy",
    "sys_trace_drain",
    "sys_trace_attach_process",
    "sys_trace_allocate_buffer",
    "sys_trace_free_buffer",
    "sys_trace_create2",
];

/// `CELL_ENOSYS` byte-exato — only error these syscalls return.
pub const CELL_ENOSYS: CellError = CellError(0x8001_0001);

#[derive(Debug, Default)]
pub struct SysTrace {
    pub create_calls: u64,
    pub start_calls: u64,
    pub stop_calls: u64,
    pub update_top_index_calls: u64,
    pub destroy_calls: u64,
    pub drain_calls: u64,
    pub attach_process_calls: u64,
    pub allocate_buffer_calls: u64,
    pub free_buffer_calls: u64,
    pub create2_calls: u64,
}

impl SysTrace {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(&mut self) -> Result<(), CellError> {
        self.create_calls = self.create_calls.saturating_add(1);
        Err(CELL_ENOSYS)
    }

    pub fn start(&mut self) -> Result<(), CellError> {
        self.start_calls = self.start_calls.saturating_add(1);
        Err(CELL_ENOSYS)
    }

    pub fn stop(&mut self) -> Result<(), CellError> {
        self.stop_calls = self.stop_calls.saturating_add(1);
        Err(CELL_ENOSYS)
    }

    pub fn update_top_index(&mut self) -> Result<(), CellError> {
        self.update_top_index_calls = self.update_top_index_calls.saturating_add(1);
        Err(CELL_ENOSYS)
    }

    pub fn destroy(&mut self) -> Result<(), CellError> {
        self.destroy_calls = self.destroy_calls.saturating_add(1);
        Err(CELL_ENOSYS)
    }

    pub fn drain(&mut self) -> Result<(), CellError> {
        self.drain_calls = self.drain_calls.saturating_add(1);
        Err(CELL_ENOSYS)
    }

    pub fn attach_process(&mut self) -> Result<(), CellError> {
        self.attach_process_calls = self.attach_process_calls.saturating_add(1);
        Err(CELL_ENOSYS)
    }

    pub fn allocate_buffer(&mut self) -> Result<(), CellError> {
        self.allocate_buffer_calls = self.allocate_buffer_calls.saturating_add(1);
        Err(CELL_ENOSYS)
    }

    pub fn free_buffer(&mut self) -> Result<(), CellError> {
        self.free_buffer_calls = self.free_buffer_calls.saturating_add(1);
        Err(CELL_ENOSYS)
    }

    pub fn create2(&mut self) -> Result<(), CellError> {
        self.create2_calls = self.create2_calls.saturating_add(1);
        Err(CELL_ENOSYS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entries() {
        assert_eq!(MODULE_NAME, "sys_trace");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 10);
        assert_eq!(REGISTERED_ENTRY_POINTS[9], "sys_trace_create2");
    }

    #[test]
    fn enosys_byte_exact() {
        assert_eq!(CELL_ENOSYS.0, 0x8001_0001);
    }

    #[test]
    fn all_syscalls_return_enosys() {
        let mut m = SysTrace::new();
        assert_eq!(m.create(), Err(CELL_ENOSYS));
        assert_eq!(m.start(), Err(CELL_ENOSYS));
        assert_eq!(m.stop(), Err(CELL_ENOSYS));
        assert_eq!(m.update_top_index(), Err(CELL_ENOSYS));
        assert_eq!(m.destroy(), Err(CELL_ENOSYS));
        assert_eq!(m.drain(), Err(CELL_ENOSYS));
        assert_eq!(m.attach_process(), Err(CELL_ENOSYS));
        assert_eq!(m.allocate_buffer(), Err(CELL_ENOSYS));
        assert_eq!(m.free_buffer(), Err(CELL_ENOSYS));
        assert_eq!(m.create2(), Err(CELL_ENOSYS));
    }

    #[test]
    fn counters_track_each_syscall_independently() {
        let mut m = SysTrace::new();
        let _ = m.create();
        let _ = m.create();
        let _ = m.start();
        let _ = m.destroy();
        let _ = m.create2();
        let _ = m.create2();
        let _ = m.create2();
        assert_eq!(m.create_calls, 2);
        assert_eq!(m.start_calls, 1);
        assert_eq!(m.destroy_calls, 1);
        assert_eq!(m.create2_calls, 3);
    }

    #[test]
    fn full_dex_session_attempt_smoke() {
        // Game tries DEX-style trace lifecycle — every step ENOSYS on retail.
        let mut m = SysTrace::new();
        for op in 0..10 {
            let r = match op {
                0 => m.create(),
                1 => m.allocate_buffer(),
                2 => m.attach_process(),
                3 => m.start(),
                4 => m.update_top_index(),
                5 => m.drain(),
                6 => m.stop(),
                7 => m.free_buffer(),
                8 => m.destroy(),
                9 => m.create2(),
                _ => unreachable!(),
            };
            assert_eq!(r, Err(CELL_ENOSYS));
        }
    }
}
