//! `rpcs3-hle-cellsysconf` — Bluetooth / system configuration HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellSysconf.cpp`. Exposes the PS3
//! XMB-level bluetooth settings: games can enumerate paired BT audio
//! headsets and HID controllers, and register an AbortCb callback so
//! the shell can dismiss long-running operations.
//!
//! ## Entry points covered
//!
//! | HLE function                       | Rust wrapper                         |
//! |------------------------------------|--------------------------------------|
//! | `cellSysconfBtGetDeviceList`       | [`SysconfManager::bt_device_list`]   |
//! | `cellSysconfAbortCb`               | [`SysconfManager::abort_cb`]         |
//! | `cellSysconfOpen`                  | [`SysconfManager::open`]             |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellSysconf.h:33-36
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const PARAM: CellError = CellError(0x8002_bb01);
}

// =====================================================================
// Constants
// =====================================================================

/// `CellSysconfBtDeviceList.device[16]`.
pub const BT_DEVICE_LIST_CAPACITY: usize = 16;

/// `CellSysconfBtDeviceInfo.name` field.
pub const BT_DEVICE_NAME_SIZE: usize = 64;

// Device types (cellSysconf.h:21-25)
pub const BT_DEVICE_TYPE_AUDIO: i32 = 0x0000_0001;
pub const BT_DEVICE_TYPE_HID: i32 = 0x0000_0002;

#[must_use]
pub fn is_known_device_type(t: i32) -> bool {
    matches!(t, BT_DEVICE_TYPE_AUDIO | BT_DEVICE_TYPE_HID)
}

// Device states (cellSysconf.h:27-31)
pub const BT_DEVICE_STATE_UNAVAILABLE: i32 = 0;
pub const BT_DEVICE_STATE_AVAILABLE: i32 = 1;

// =====================================================================
// Domain types
// =====================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BtDeviceInfo {
    pub device_id: u64,
    pub device_type: i32,
    pub state: i32,
    pub name: String, // ≤ BT_DEVICE_NAME_SIZE chars (truncated on insert if longer)
}

impl BtDeviceInfo {
    #[must_use]
    pub fn new(device_id: u64, device_type: i32, name: impl Into<String>) -> Self {
        Self { device_id, device_type, state: BT_DEVICE_STATE_AVAILABLE, name: name.into() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BtDeviceList {
    pub devices: Vec<BtDeviceInfo>, // ≤ BT_DEVICE_LIST_CAPACITY
}

// =====================================================================
// SysconfManager — backing store for the XMB BT device table
// =====================================================================

#[derive(Clone, Debug)]
pub struct SysconfManager {
    devices: Vec<BtDeviceInfo>,
    abort_cb_registered: bool,
    open_handle: bool,
}

impl SysconfManager {
    #[must_use]
    pub fn new() -> Self {
        Self { devices: Vec::new(), abort_cb_registered: false, open_handle: false }
    }

    /// Register a BT device in the paired-device table. Mirrors the
    /// XMB-side action of the user completing a "Register New Device"
    /// flow. Games cannot do this via the HLE API; it's an admin-side
    /// helper for tests / backend integration.
    pub fn register_device(&mut self, device: BtDeviceInfo) -> Result<(), CellError> {
        if self.devices.len() >= BT_DEVICE_LIST_CAPACITY {
            return Err(errors::PARAM);
        }
        if !is_known_device_type(device.device_type) {
            return Err(errors::PARAM);
        }
        if device.device_id == 0 {
            return Err(errors::PARAM);
        }
        if device.name.len() >= BT_DEVICE_NAME_SIZE {
            return Err(errors::PARAM);
        }
        if self.devices.iter().any(|d| d.device_id == device.device_id) {
            return Err(errors::PARAM);
        }
        self.devices.push(device);
        Ok(())
    }

    pub fn unregister_device(&mut self, device_id: u64) -> Result<(), CellError> {
        let pos = self.devices.iter().position(|d| d.device_id == device_id).ok_or(errors::PARAM)?;
        self.devices.remove(pos);
        Ok(())
    }

    /// Emulate a device toggle between available / unavailable — games
    /// observe this via subsequent `bt_device_list` calls.
    pub fn set_device_state(&mut self, device_id: u64, state: i32) -> Result<(), CellError> {
        if !matches!(state, BT_DEVICE_STATE_UNAVAILABLE | BT_DEVICE_STATE_AVAILABLE) {
            return Err(errors::PARAM);
        }
        let d = self.devices.iter_mut().find(|d| d.device_id == device_id).ok_or(errors::PARAM)?;
        d.state = state;
        Ok(())
    }

    // ----------------- Queries -----------------

    /// `cellSysconfBtGetDeviceList(deviceList)`. Returns (up to
    /// `BT_DEVICE_LIST_CAPACITY`) the current table.
    pub fn bt_device_list(&self) -> Result<BtDeviceList, CellError> {
        let take = self.devices.len().min(BT_DEVICE_LIST_CAPACITY);
        Ok(BtDeviceList { devices: self.devices[..take].to_vec() })
    }

    /// Filter variant — returns only devices matching `device_type`.
    pub fn bt_device_list_filtered(&self, device_type: i32) -> Result<BtDeviceList, CellError> {
        if !is_known_device_type(device_type) {
            return Err(errors::PARAM);
        }
        Ok(BtDeviceList {
            devices: self.devices.iter().filter(|d| d.device_type == device_type).cloned().collect(),
        })
    }

    // ----------------- AbortCb -----------------

    /// `cellSysconfAbortCb(callback)`. Registers an abort-hook the shell
    /// calls when the user dismisses a modal dialog. We just track the
    /// registration state here.
    pub fn abort_cb(&mut self) -> Result<(), CellError> {
        if self.abort_cb_registered {
            return Err(errors::PARAM);
        }
        self.abort_cb_registered = true;
        Ok(())
    }

    pub fn clear_abort_cb(&mut self) {
        self.abort_cb_registered = false;
    }

    #[must_use]
    pub fn is_abort_cb_registered(&self) -> bool {
        self.abort_cb_registered
    }

    // ----------------- Open / Close -----------------

    /// `cellSysconfOpen(type, callback, userdata, extParam, userNumber)`
    /// in the cellSysconfEx API. We model it as a single-slot handle.
    pub fn open(&mut self) -> Result<(), CellError> {
        if self.open_handle {
            return Err(errors::PARAM);
        }
        self.open_handle = true;
        Ok(())
    }

    pub fn close(&mut self) -> Result<(), CellError> {
        if !self.open_handle {
            return Err(errors::PARAM);
        }
        self.open_handle = false;
        Ok(())
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open_handle
    }

    #[must_use]
    pub fn device_count(&self) -> usize {
        self.devices.len()
    }
}

impl Default for SysconfManager {
    fn default() -> Self {
        Self::new()
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn audio_device(id: u64, name: &str) -> BtDeviceInfo {
        BtDeviceInfo::new(id, BT_DEVICE_TYPE_AUDIO, name)
    }

    fn hid_device(id: u64, name: &str) -> BtDeviceInfo {
        BtDeviceInfo::new(id, BT_DEVICE_TYPE_HID, name)
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::PARAM.0, 0x8002_bb01);
    }

    #[test]
    fn constants_stable() {
        assert_eq!(BT_DEVICE_LIST_CAPACITY, 16);
        assert_eq!(BT_DEVICE_NAME_SIZE, 64);
        assert_eq!(BT_DEVICE_TYPE_AUDIO, 0x0000_0001);
        assert_eq!(BT_DEVICE_TYPE_HID, 0x0000_0002);
        assert_eq!(BT_DEVICE_STATE_UNAVAILABLE, 0);
        assert_eq!(BT_DEVICE_STATE_AVAILABLE, 1);
    }

    #[test]
    fn empty_device_list() {
        let m = SysconfManager::new();
        assert_eq!(m.bt_device_list().unwrap().devices.len(), 0);
    }

    #[test]
    fn register_device_happy_path() {
        let mut m = SysconfManager::new();
        m.register_device(audio_device(1, "Headset")).unwrap();
        assert_eq!(m.device_count(), 1);
        let list = m.bt_device_list().unwrap();
        assert_eq!(list.devices[0].name, "Headset");
        assert_eq!(list.devices[0].device_type, BT_DEVICE_TYPE_AUDIO);
    }

    #[test]
    fn register_device_zero_id_rejected() {
        let mut m = SysconfManager::new();
        assert_eq!(m.register_device(audio_device(0, "Zero")), Err(errors::PARAM));
    }

    #[test]
    fn register_device_unknown_type_rejected() {
        let mut m = SysconfManager::new();
        let bad = BtDeviceInfo::new(1, 99, "X");
        assert_eq!(m.register_device(bad), Err(errors::PARAM));
    }

    #[test]
    fn register_device_name_too_long_rejected() {
        let mut m = SysconfManager::new();
        let long_name = "a".repeat(BT_DEVICE_NAME_SIZE);
        assert_eq!(m.register_device(audio_device(1, &long_name)), Err(errors::PARAM));
    }

    #[test]
    fn register_device_duplicate_id_rejected() {
        let mut m = SysconfManager::new();
        m.register_device(audio_device(1, "A")).unwrap();
        assert_eq!(m.register_device(audio_device(1, "B")), Err(errors::PARAM));
    }

    #[test]
    fn register_device_over_capacity_rejected() {
        let mut m = SysconfManager::new();
        for i in 1..=(BT_DEVICE_LIST_CAPACITY as u64) {
            m.register_device(audio_device(i, &format!("D{i}"))).unwrap();
        }
        assert_eq!(m.register_device(audio_device(99, "Extra")), Err(errors::PARAM));
    }

    #[test]
    fn unregister_device_happy_path() {
        let mut m = SysconfManager::new();
        m.register_device(audio_device(1, "A")).unwrap();
        m.unregister_device(1).unwrap();
        assert_eq!(m.device_count(), 0);
    }

    #[test]
    fn unregister_device_unknown_rejected() {
        let mut m = SysconfManager::new();
        assert_eq!(m.unregister_device(99), Err(errors::PARAM));
    }

    #[test]
    fn set_device_state_toggles() {
        let mut m = SysconfManager::new();
        m.register_device(audio_device(1, "A")).unwrap();
        m.set_device_state(1, BT_DEVICE_STATE_UNAVAILABLE).unwrap();
        let list = m.bt_device_list().unwrap();
        assert_eq!(list.devices[0].state, BT_DEVICE_STATE_UNAVAILABLE);
        m.set_device_state(1, BT_DEVICE_STATE_AVAILABLE).unwrap();
        assert_eq!(m.bt_device_list().unwrap().devices[0].state, BT_DEVICE_STATE_AVAILABLE);
    }

    #[test]
    fn set_device_state_bad_state_rejected() {
        let mut m = SysconfManager::new();
        m.register_device(audio_device(1, "A")).unwrap();
        assert_eq!(m.set_device_state(1, 99), Err(errors::PARAM));
    }

    #[test]
    fn set_device_state_unknown_device_rejected() {
        let mut m = SysconfManager::new();
        assert_eq!(m.set_device_state(999, BT_DEVICE_STATE_AVAILABLE), Err(errors::PARAM));
    }

    #[test]
    fn bt_device_list_filtered_audio() {
        let mut m = SysconfManager::new();
        m.register_device(audio_device(1, "Headset")).unwrap();
        m.register_device(hid_device(2, "Controller")).unwrap();
        m.register_device(audio_device(3, "Speaker")).unwrap();
        let audio = m.bt_device_list_filtered(BT_DEVICE_TYPE_AUDIO).unwrap();
        assert_eq!(audio.devices.len(), 2);
        assert!(audio.devices.iter().all(|d| d.device_type == BT_DEVICE_TYPE_AUDIO));
    }

    #[test]
    fn bt_device_list_filtered_hid() {
        let mut m = SysconfManager::new();
        m.register_device(audio_device(1, "Headset")).unwrap();
        m.register_device(hid_device(2, "DS3")).unwrap();
        let hid = m.bt_device_list_filtered(BT_DEVICE_TYPE_HID).unwrap();
        assert_eq!(hid.devices.len(), 1);
        assert_eq!(hid.devices[0].device_id, 2);
    }

    #[test]
    fn bt_device_list_filtered_unknown_type_rejected() {
        let m = SysconfManager::new();
        assert_eq!(m.bt_device_list_filtered(99), Err(errors::PARAM));
    }

    #[test]
    fn abort_cb_register_cycle() {
        let mut m = SysconfManager::new();
        assert!(!m.is_abort_cb_registered());
        m.abort_cb().unwrap();
        assert!(m.is_abort_cb_registered());
        // Re-register is rejected.
        assert_eq!(m.abort_cb(), Err(errors::PARAM));
        m.clear_abort_cb();
        assert!(!m.is_abort_cb_registered());
        m.abort_cb().unwrap();
    }

    #[test]
    fn open_close_cycle() {
        let mut m = SysconfManager::new();
        assert!(!m.is_open());
        m.open().unwrap();
        assert!(m.is_open());
        assert_eq!(m.open(), Err(errors::PARAM));
        m.close().unwrap();
        assert!(!m.is_open());
        assert_eq!(m.close(), Err(errors::PARAM));
    }

    #[test]
    fn full_lifecycle_smoke() {
        let mut m = SysconfManager::new();
        m.register_device(audio_device(0x123, "Sony Wireless Headset")).unwrap();
        m.register_device(hid_device(0x456, "DualShock 3")).unwrap();
        m.set_device_state(0x123, BT_DEVICE_STATE_UNAVAILABLE).unwrap();
        m.open().unwrap();
        m.abort_cb().unwrap();
        let list = m.bt_device_list().unwrap();
        assert_eq!(list.devices.len(), 2);
        let hid_only = m.bt_device_list_filtered(BT_DEVICE_TYPE_HID).unwrap();
        assert_eq!(hid_only.devices.len(), 1);
        m.unregister_device(0x123).unwrap();
        m.close().unwrap();
    }

    #[test]
    fn device_list_truncates_at_capacity() {
        // Registration is capped at BT_DEVICE_LIST_CAPACITY, so device_list
        // should always return ≤ 16. Prove the upper bound is enforced.
        let mut m = SysconfManager::new();
        for i in 1..=(BT_DEVICE_LIST_CAPACITY as u64) {
            m.register_device(audio_device(i, &format!("D{i}"))).unwrap();
        }
        assert_eq!(m.bt_device_list().unwrap().devices.len(), BT_DEVICE_LIST_CAPACITY);
    }

    #[test]
    fn is_known_device_type_helper() {
        assert!(is_known_device_type(BT_DEVICE_TYPE_AUDIO));
        assert!(is_known_device_type(BT_DEVICE_TYPE_HID));
        assert!(!is_known_device_type(0));
        assert!(!is_known_device_type(3));
        assert!(!is_known_device_type(99));
    }
}
