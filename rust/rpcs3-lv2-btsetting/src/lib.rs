//! Rust port of `rpcs3/Emu/Cell/lv2/sys_btsetting.cpp` — PS3 LV2 Bluetooth
//! settings interface syscall (1 entry, 13 lines C++).
//!
//! `sys_btsetting_if(cmd, msg)` — XMB Bluetooth settings ioctl-style
//! interface. The upstream is a `todo()` stub returning CELL_OK.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sys_btsetting";

pub const REGISTERED_ENTRY_POINTS: &[&str] = &["sys_btsetting_if"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtsettingCall {
    pub cmd: u64,
    pub msg_addr: u32,
}

#[derive(Debug, Default)]
pub struct SysBtsetting {
    pub calls: Vec<BtsettingCall>,
    pub if_calls: u64,
}

impl SysBtsetting {
    pub fn new() -> Self {
        Self::default()
    }

    /// `sys_btsetting_if(cmd, msg)` — upstream stub returning CELL_OK.
    pub fn btsetting_if(&mut self, cmd: u64, msg_addr: u32) -> Result<(), CellError> {
        self.if_calls = self.if_calls.saturating_add(1);
        self.calls.push(BtsettingCall { cmd, msg_addr });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entry() {
        assert_eq!(MODULE_NAME, "sys_btsetting");
        assert_eq!(REGISTERED_ENTRY_POINTS, &["sys_btsetting_if"]);
    }

    #[test]
    fn btsetting_if_returns_ok() {
        let mut m = SysBtsetting::new();
        m.btsetting_if(0xCAFE, 0x4000_0000).unwrap();
        assert_eq!(m.if_calls, 1);
    }

    #[test]
    fn calls_recorded() {
        let mut m = SysBtsetting::new();
        m.btsetting_if(1, 0x100).unwrap();
        m.btsetting_if(2, 0x200).unwrap();
        assert_eq!(m.calls.len(), 2);
        assert_eq!(m.calls[0].cmd, 1);
        assert_eq!(m.calls[1].msg_addr, 0x200);
    }

    #[test]
    fn null_msg_addr_accepted() {
        let mut m = SysBtsetting::new();
        m.btsetting_if(0, 0).unwrap();
        assert_eq!(m.calls[0].msg_addr, 0);
    }

    #[test]
    fn many_calls_counted() {
        let mut m = SysBtsetting::new();
        for i in 0..100 {
            m.btsetting_if(i, 0).unwrap();
        }
        assert_eq!(m.if_calls, 100);
    }
}
