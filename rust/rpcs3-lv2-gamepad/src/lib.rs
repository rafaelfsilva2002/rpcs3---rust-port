//! Rust port of `rpcs3/Emu/Cell/lv2/sys_gamepad.cpp` — PS3 LV2 gamepad
//! ycon dispatcher syscall (10 internal handlers + 1 packet dispatcher).
//!
//! Single observable syscall (621 / 0x26D) is `sys_gamepad_ycon_if(packet_id, in, out)`
//! which dispatches to 10 internal handlers based on packet_id. All
//! handlers and the dispatcher return CELL_OK regardless. Unknown
//! packet_id is logged but still returns CELL_OK (cpp:92-95).
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sys_gamepad";

/// 11 entries: 10 internal handlers + the packet dispatcher.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sys_gamepad_ycon_initalize", // typo preserved from cpp:7 ("initalize")
    "sys_gamepad_ycon_finalize",
    "sys_gamepad_ycon_has_input_ownership",
    "sys_gamepad_ycon_enumerate_device",
    "sys_gamepad_ycon_get_device_info",
    "sys_gamepad_ycon_read_raw_report",
    "sys_gamepad_ycon_write_raw_report",
    "sys_gamepad_ycon_get_feature",
    "sys_gamepad_ycon_set_feature",
    "sys_gamepad_ycon_is_gem",
    "sys_gamepad_ycon_if",
];

#[derive(Debug, Default)]
pub struct SysGamepad {
    pub initalize_calls: u64,
    pub finalize_calls: u64,
    pub has_input_ownership_calls: u64,
    pub enumerate_device_calls: u64,
    pub get_device_info_calls: u64,
    pub read_raw_report_calls: u64,
    pub write_raw_report_calls: u64,
    pub get_feature_calls: u64,
    pub set_feature_calls: u64,
    pub is_gem_calls: u64,
    pub if_calls: u64,
    pub unknown_packet_count: u64,
}

impl SysGamepad {
    pub fn new() -> Self {
        Self::default()
    }

    /// `sys_gamepad_ycon_if(packet_id, in, out)` — dispatches to internal
    /// handlers based on packet_id (0..=9 known, others log + return OK).
    pub fn ycon_if(&mut self, packet_id: u8) -> Result<(), CellError> {
        self.if_calls = self.if_calls.saturating_add(1);
        match packet_id {
            0 => {
                self.initalize_calls = self.initalize_calls.saturating_add(1);
            }
            1 => {
                self.finalize_calls = self.finalize_calls.saturating_add(1);
            }
            2 => {
                self.has_input_ownership_calls =
                    self.has_input_ownership_calls.saturating_add(1);
            }
            3 => {
                self.enumerate_device_calls = self.enumerate_device_calls.saturating_add(1);
            }
            4 => {
                self.get_device_info_calls = self.get_device_info_calls.saturating_add(1);
            }
            5 => {
                self.read_raw_report_calls = self.read_raw_report_calls.saturating_add(1);
            }
            6 => {
                self.write_raw_report_calls = self.write_raw_report_calls.saturating_add(1);
            }
            7 => {
                self.get_feature_calls = self.get_feature_calls.saturating_add(1);
            }
            8 => {
                self.set_feature_calls = self.set_feature_calls.saturating_add(1);
            }
            9 => {
                self.is_gem_calls = self.is_gem_calls.saturating_add(1);
            }
            _ => {
                self.unknown_packet_count = self.unknown_packet_count.saturating_add(1);
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
        assert_eq!(MODULE_NAME, "sys_gamepad");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 11);
        // Preserve C++ typo cpp:7
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "sys_gamepad_ycon_initalize");
    }

    #[test]
    fn dispatch_each_packet_id() {
        let mut m = SysGamepad::new();
        for id in 0..=9 {
            m.ycon_if(id).unwrap();
        }
        assert_eq!(m.initalize_calls, 1);
        assert_eq!(m.finalize_calls, 1);
        assert_eq!(m.has_input_ownership_calls, 1);
        assert_eq!(m.enumerate_device_calls, 1);
        assert_eq!(m.get_device_info_calls, 1);
        assert_eq!(m.read_raw_report_calls, 1);
        assert_eq!(m.write_raw_report_calls, 1);
        assert_eq!(m.get_feature_calls, 1);
        assert_eq!(m.set_feature_calls, 1);
        assert_eq!(m.is_gem_calls, 1);
        assert_eq!(m.if_calls, 10);
        assert_eq!(m.unknown_packet_count, 0);
    }

    #[test]
    fn unknown_packet_logs_but_returns_ok() {
        let mut m = SysGamepad::new();
        m.ycon_if(50).unwrap();
        m.ycon_if(255).unwrap();
        assert_eq!(m.unknown_packet_count, 2);
        assert_eq!(m.if_calls, 2);
    }

    #[test]
    fn boundary_packet_ids() {
        let mut m = SysGamepad::new();
        m.ycon_if(9).unwrap(); // last known
        m.ycon_if(10).unwrap(); // first unknown
        assert_eq!(m.is_gem_calls, 1);
        assert_eq!(m.unknown_packet_count, 1);
    }
}
