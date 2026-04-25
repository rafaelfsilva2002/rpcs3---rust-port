//! Rust port of `rpcs3/Emu/Cell/lv2/sys_console.cpp` — PS3 LV2 system
//! console write syscall (1 entry, 14 lines C++).
//!
//! `sys_console_write(buf, len)` — generic kernel-side console output.
//! Distinct from sysPrxForUser `console_write` which is a user-mode shim.
//! Upstream is a `todo()` stub returning CELL_OK regardless.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sys_console";

pub const REGISTERED_ENTRY_POINTS: &[&str] = &["sys_console_write"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleWrite {
    pub buf_addr: u32,
    pub len: u32,
    /// Captured payload (may be empty when caller passed null).
    pub payload: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct SysConsole {
    pub writes: Vec<ConsoleWrite>,
    pub write_calls: u64,
    pub bytes_written: u64,
}

impl SysConsole {
    pub fn new() -> Self {
        Self::default()
    }

    /// `sys_console_write(buf, len)` — upstream stub returning CELL_OK
    /// regardless of args. The Rust port captures the payload (when caller
    /// supplies one) for tracing/test introspection.
    pub fn write(
        &mut self,
        buf_addr: u32,
        len: u32,
        payload: Option<&[u8]>,
    ) -> Result<(), CellError> {
        self.write_calls = self.write_calls.saturating_add(1);
        let captured = payload.map(|p| p.to_vec()).unwrap_or_default();
        self.bytes_written = self.bytes_written.saturating_add(len as u64);
        self.writes.push(ConsoleWrite {
            buf_addr,
            len,
            payload: captured,
        });
        Ok(())
    }

    /// Concatenate all captured payloads as UTF-8 (lossy).
    pub fn captured_text(&self) -> String {
        let mut s = String::new();
        for w in &self.writes {
            if let Ok(text) = core::str::from_utf8(&w.payload) {
                s.push_str(text);
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entry() {
        assert_eq!(MODULE_NAME, "sys_console");
        assert_eq!(REGISTERED_ENTRY_POINTS, &["sys_console_write"]);
    }

    #[test]
    fn write_returns_ok_with_payload() {
        let mut m = SysConsole::new();
        m.write(0x4000_0000, 5, Some(b"hello")).unwrap();
        assert_eq!(m.write_calls, 1);
        assert_eq!(m.bytes_written, 5);
        assert_eq!(m.writes[0].payload, b"hello");
    }

    #[test]
    fn null_payload_accepted_with_zero_len() {
        let mut m = SysConsole::new();
        m.write(0, 0, None).unwrap();
        assert_eq!(m.writes[0].payload.len(), 0);
        assert_eq!(m.bytes_written, 0);
    }

    #[test]
    fn captured_text_concatenates() {
        let mut m = SysConsole::new();
        m.write(0x100, 5, Some(b"hello")).unwrap();
        m.write(0x200, 1, Some(b" ")).unwrap();
        m.write(0x300, 5, Some(b"world")).unwrap();
        assert_eq!(m.captured_text(), "hello world");
    }

    #[test]
    fn bytes_written_accumulates() {
        let mut m = SysConsole::new();
        m.write(0, 100, None).unwrap();
        m.write(0, 200, None).unwrap();
        m.write(0, 50, None).unwrap();
        assert_eq!(m.bytes_written, 350);
    }

    #[test]
    fn many_writes_tracked() {
        let mut m = SysConsole::new();
        for _ in 0..40 {
            m.write(0, 1, Some(b"x")).unwrap();
        }
        assert_eq!(m.write_calls, 40);
        assert_eq!(m.writes.len(), 40);
    }
}
