//! Rust port of `rpcs3/Emu/Cell/lv2/sys_hid.cpp` — PS3 LV2 HID manager
//! syscalls (8 entries, 191 lines C++).
//!
//! `sys_hid_manager_open/ioctl/check_focus/513/514/is_process_permission_root/
//! add_hot_key_observer/read`. Several entries have observable behaviour
//! preserved byte-exato from the realhw syscall dump in cpp:48-67:
//!
//! * `open(device_type, port_no)` — handle starts at 0x100, increments per
//!   call (cpp:31-32). device_type ∈ {0,1,2,3} (1=pad, 2=kb, 3=mouse).
//! * `ioctl(handle, pkg_id=2, buf)` — fills `sys_hid_info_2` with VID=0x054C
//!   PID=0x0268 + 17 bytes of realhw dump (cpp:73-78).
//! * `ioctl(handle, pkg_id=5, buf)` — fills `sys_hid_info_5` with same VID/PID.
//! * `check_focus()` — `not_an_error(1)` (cpp:100).
//! * `is_process_permission_root(pid)` — returns 1 iff caller has root perm.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sys_hid";

pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sys_hid_manager_open",
    "sys_hid_manager_ioctl",
    "sys_hid_manager_check_focus",
    "sys_hid_manager_513",
    "sys_hid_manager_514",
    "sys_hid_manager_is_process_permission_root",
    "sys_hid_manager_add_hot_key_observer",
    "sys_hid_manager_read",
];

pub const CELL_EINVAL: CellError = CellError(0x8001_0002);
pub const CELL_EFAULT: CellError = CellError(0x8001_000D);

/// Initial handle value byte-exato cpp:31 `static u32 ctr = 0x100`.
pub const HID_HANDLE_INITIAL: u32 = 0x100;

/// Realhw VID byte-exato cpp:74/83 — Sony Corp.
pub const SONY_VID: u16 = 0x054C;
/// Realhw PID byte-exato cpp:75/84 — DualShock 3.
pub const DUALSHOCK3_PID: u16 = 0x0268;

/// 17-byte realhw dump for ioctl pkg_id=2 (cpp:77).
pub const IOCTL_PKG_ID_2_REALHW_TAIL: [u8; 17] = [
    0x01, 0x02, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x03, 0x50, 0x00, 0x00, 0x1c,
    0x1f,
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SysHidInfo2 {
    pub vid: u16,
    pub pid: u16,
    pub unk: [u8; 17],
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SysHidInfo5 {
    pub vid: u16,
    pub pid: u16,
}

#[derive(Debug, Default)]
pub struct SysHid {
    pub next_handle: u32,
    pub open_handles: Vec<u32>,
    pub hot_key_observers: Vec<u32>,
    pub root_perm: bool,

    pub open_calls: u64,
    pub ioctl_calls: u64,
    pub check_focus_calls: u64,
    pub call_513_calls: u64,
    pub call_514_calls: u64,
    pub is_root_calls: u64,
    pub add_hot_key_calls: u64,
    pub read_calls: u64,
}

impl SysHid {
    pub fn new() -> Self {
        Self {
            next_handle: HID_HANDLE_INITIAL,
            ..Default::default()
        }
    }

    /// `sys_hid_manager_open(device_type, port_no, handle)` cpp:14-41.
    /// device_type > 3 = EINVAL, null handle = EFAULT, otherwise allocate
    /// monotonic handle starting at 0x100.
    pub fn open(
        &mut self,
        device_type: u64,
        _port_no: u64,
        handle_out: Option<&mut u32>,
    ) -> Result<(), CellError> {
        self.open_calls = self.open_calls.saturating_add(1);
        if device_type > 3 {
            return Err(CELL_EINVAL);
        }
        let slot = handle_out.ok_or(CELL_EFAULT)?;
        let h = self.next_handle;
        self.next_handle = self.next_handle.wrapping_add(1);
        *slot = h;
        self.open_handles.push(h);
        Ok(())
    }

    /// `sys_hid_manager_ioctl(handle, pkg_id, buf, buf_size)` cpp:43-94.
    pub fn ioctl_pkg_id_2(&mut self, _handle: u32) -> Result<SysHidInfo2, CellError> {
        self.ioctl_calls = self.ioctl_calls.saturating_add(1);
        Ok(SysHidInfo2 {
            vid: SONY_VID,
            pid: DUALSHOCK3_PID,
            unk: IOCTL_PKG_ID_2_REALHW_TAIL,
        })
    }

    pub fn ioctl_pkg_id_5(&mut self, _handle: u32) -> Result<SysHidInfo5, CellError> {
        self.ioctl_calls = self.ioctl_calls.saturating_add(1);
        Ok(SysHidInfo5 {
            vid: SONY_VID,
            pid: DUALSHOCK3_PID,
        })
    }

    /// `sys_hid_manager_check_focus()` cpp:96-101 — `not_an_error(1)`.
    pub fn check_focus(&mut self) -> Result<u32, CellError> {
        self.check_focus_calls = self.check_focus_calls.saturating_add(1);
        Ok(1)
    }

    pub fn manager_513(&mut self) -> Result<(), CellError> {
        self.call_513_calls = self.call_513_calls.saturating_add(1);
        Ok(())
    }

    pub fn manager_514(&mut self, _pkg_id: u32) -> Result<(), CellError> {
        self.call_514_calls = self.call_514_calls.saturating_add(1);
        Ok(())
    }

    /// `sys_hid_manager_is_process_permission_root(pid)` cpp:140-145.
    pub fn is_process_permission_root(&mut self, _pid: u32) -> Result<u32, CellError> {
        self.is_root_calls = self.is_root_calls.saturating_add(1);
        Ok(if self.root_perm { 1 } else { 0 })
    }

    pub fn add_hot_key_observer(
        &mut self,
        event_queue: u32,
        _unk_out: Option<&mut u32>,
    ) -> Result<(), CellError> {
        self.add_hot_key_calls = self.add_hot_key_calls.saturating_add(1);
        self.hot_key_observers.push(event_queue);
        Ok(())
    }

    /// `sys_hid_manager_read(handle, pkg_id, buf, buf_size)` cpp:154-191.
    /// null buf = EFAULT (cpp:156-159).
    pub fn read(
        &mut self,
        _handle: u32,
        _pkg_id: u32,
        buf_present: bool,
        _buf_size: u64,
    ) -> Result<u32, CellError> {
        self.read_calls = self.read_calls.saturating_add(1);
        if !buf_present {
            return Err(CELL_EFAULT);
        }
        // No real CellPadData backing — return 0 bytes copied.
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn module_and_entries() {
        assert_eq!(MODULE_NAME, "sys_hid");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 8);
    }

    #[test]
    fn handle_starts_at_0x100() {
        assert_eq!(HID_HANDLE_INITIAL, 0x100);
        let mut m = SysHid::new();
        let mut h = 0u32;
        m.open(1, 0, Some(&mut h)).unwrap();
        assert_eq!(h, 0x100);
        m.open(1, 0, Some(&mut h)).unwrap();
        assert_eq!(h, 0x101);
    }

    #[test]
    fn open_invalid_device_type() {
        let mut m = SysHid::new();
        let mut h = 0u32;
        assert_eq!(m.open(4, 0, Some(&mut h)), Err(CELL_EINVAL));
        assert_eq!(m.open(99, 0, Some(&mut h)), Err(CELL_EINVAL));
        // 0..=3 all OK.
        for dt in 0..=3 {
            m.open(dt, 0, Some(&mut h)).unwrap();
        }
    }

    #[test]
    fn open_null_handle_efault() {
        let mut m = SysHid::new();
        assert_eq!(m.open(1, 0, None), Err(CELL_EFAULT));
    }

    #[test]
    fn vid_pid_byte_exact() {
        assert_eq!(SONY_VID, 0x054C);
        assert_eq!(DUALSHOCK3_PID, 0x0268);
    }

    #[test]
    fn ioctl_pkg_id_2_returns_realhw_dump() {
        let mut m = SysHid::new();
        let info = m.ioctl_pkg_id_2(0x100).unwrap();
        assert_eq!(info.vid, 0x054C);
        assert_eq!(info.pid, 0x0268);
        assert_eq!(info.unk, IOCTL_PKG_ID_2_REALHW_TAIL);
        // First byte cpp:77.
        assert_eq!(info.unk[0], 0x01);
        // Last byte cpp:77.
        assert_eq!(info.unk[16], 0x1f);
    }

    #[test]
    fn ioctl_pkg_id_5_returns_vid_pid_only() {
        let mut m = SysHid::new();
        let info = m.ioctl_pkg_id_5(0x100).unwrap();
        assert_eq!(info.vid, 0x054C);
        assert_eq!(info.pid, 0x0268);
    }

    #[test]
    fn check_focus_returns_one() {
        let mut m = SysHid::new();
        assert_eq!(m.check_focus(), Ok(1));
    }

    #[test]
    fn is_process_permission_root() {
        let mut m = SysHid::new();
        assert_eq!(m.is_process_permission_root(1), Ok(0));
        m.root_perm = true;
        assert_eq!(m.is_process_permission_root(1), Ok(1));
    }

    #[test]
    fn add_hot_key_observer_tracks() {
        let mut m = SysHid::new();
        m.add_hot_key_observer(0xCAFE, None).unwrap();
        m.add_hot_key_observer(0xBEEF, None).unwrap();
        assert_eq!(m.hot_key_observers, vec![0xCAFE, 0xBEEF]);
    }

    #[test]
    fn read_null_buf_efault() {
        let mut m = SysHid::new();
        assert_eq!(m.read(0x100, 2, false, 64), Err(CELL_EFAULT));
    }

    #[test]
    fn read_returns_zero_bytes_no_pad_data() {
        let mut m = SysHid::new();
        assert_eq!(m.read(0x100, 2, true, 64), Ok(0));
    }

    #[test]
    fn full_hid_lifecycle_smoke() {
        let mut m = SysHid::new();
        m.root_perm = true;
        let mut h = 0u32;
        m.open(1, 0, Some(&mut h)).unwrap(); // pad
        assert_eq!(h, 0x100);
        let info = m.ioctl_pkg_id_2(h).unwrap();
        assert_eq!(info.vid, SONY_VID);
        let n = m.read(h, 2, true, 64).unwrap();
        assert_eq!(n, 0);
        m.add_hot_key_observer(0xABCD, None).unwrap();
        assert_eq!(m.is_process_permission_root(1), Ok(1));
        assert_eq!(m.check_focus(), Ok(1));
        m.manager_513().unwrap();
        m.manager_514(0xE).unwrap();
    }
}
