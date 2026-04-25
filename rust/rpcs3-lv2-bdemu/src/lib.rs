//! Rust port of `rpcs3/Emu/Cell/lv2/sys_bdemu.cpp` — PS3 LV2 Blu-ray
//! emulator helper syscall (1 entry, 14 lines C++).
//!
//! `sys_bdemu_send_command(cmd, a2, a3, buf, buf_len)` — used by
//! cellBdEmu / sys_bdvd to send raw commands down to the BD device. The
//! upstream is a `todo()` stub returning CELL_OK.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sys_bdemu";

/// Single syscall registered by the LV2 BD-emulator subsystem.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &["sys_bdemu_send_command"];

/// Captured command record — useful for tests/tracing without a real BD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BdemuCommand {
    pub cmd: u64,
    pub a2: u64,
    pub a3: u64,
    pub buf_addr: u32,
    pub buf_len: u64,
}

#[derive(Debug, Default)]
pub struct SysBdemu {
    pub commands: Vec<BdemuCommand>,
    pub send_command_calls: u64,
}

impl SysBdemu {
    pub fn new() -> Self {
        Self::default()
    }

    /// `sys_bdemu_send_command(cmd, a2, a3, buf, buf_len)` — upstream stub.
    /// The Rust port records every command for test introspection but
    /// returns CELL_OK unconditionally, matching upstream behaviour.
    pub fn send_command(
        &mut self,
        cmd: u64,
        a2: u64,
        a3: u64,
        buf_addr: u32,
        buf_len: u64,
    ) -> Result<(), CellError> {
        self.send_command_calls = self.send_command_calls.saturating_add(1);
        self.commands.push(BdemuCommand {
            cmd,
            a2,
            a3,
            buf_addr,
            buf_len,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entry() {
        assert_eq!(MODULE_NAME, "sys_bdemu");
        assert_eq!(REGISTERED_ENTRY_POINTS, &["sys_bdemu_send_command"]);
    }

    #[test]
    fn send_command_returns_ok() {
        let mut m = SysBdemu::new();
        m.send_command(0x42, 0x100, 0x200, 0x4000_0000, 1024).unwrap();
        assert_eq!(m.send_command_calls, 1);
    }

    #[test]
    fn commands_are_recorded() {
        let mut m = SysBdemu::new();
        m.send_command(1, 2, 3, 0x1000, 16).unwrap();
        m.send_command(0xFFFF_FFFF_FFFF_FFFF, 0, 0, 0, 0).unwrap();
        assert_eq!(m.commands.len(), 2);
        assert_eq!(m.commands[0].cmd, 1);
        assert_eq!(m.commands[0].buf_addr, 0x1000);
        assert_eq!(m.commands[1].cmd, 0xFFFF_FFFF_FFFF_FFFF);
    }

    #[test]
    fn null_buf_and_zero_len_accepted() {
        let mut m = SysBdemu::new();
        // upstream stub doesn't validate any args.
        m.send_command(0, 0, 0, 0, 0).unwrap();
        assert_eq!(m.commands[0].buf_addr, 0);
        assert_eq!(m.commands[0].buf_len, 0);
    }

    #[test]
    fn counter_persists_across_many_calls() {
        let mut m = SysBdemu::new();
        for i in 0..50 {
            m.send_command(i, 0, 0, 0, 0).unwrap();
        }
        assert_eq!(m.send_command_calls, 50);
        assert_eq!(m.commands.len(), 50);
    }
}
