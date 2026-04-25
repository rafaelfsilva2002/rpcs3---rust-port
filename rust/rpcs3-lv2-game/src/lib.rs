//! Rust port of `rpcs3/Emu/Cell/lv2/sys_game.cpp` — PS3 LV2 game-related
//! syscalls (8 entries, 293 lines C++).
//!
//! 8 entries: watchdog start/stop/clear, sw_version set/get, board_storage
//! read/write, rtc_status. The watchdog has a 50ms tick + needs_restart
//! flag mechanism. board_storage is a 16-byte v128 atomic blob.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sys_game";

pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "_sys_game_watchdog_start",
    "_sys_game_watchdog_stop",
    "_sys_game_watchdog_clear",
    "_sys_game_set_system_sw_version",
    "_sys_game_get_system_sw_version",
    "_sys_game_board_storage_read",
    "_sys_game_board_storage_write",
    "_sys_game_get_rtc_status",
];

pub const CELL_EFAULT: CellError = CellError(0x8001_000D);
pub const CELL_ENOSYS: CellError = CellError(0x8001_0001);
pub const CELL_EABORT: CellError = CellError(0x8001_0010);

/// `board_storage` size byte-exato — sizeof(v128) = 16 bytes.
pub const BOARD_STORAGE_SIZE: usize = 16;

/// Watchdog control struct cpp:94-100.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WatchdogControl {
    pub needs_restart: bool,
    pub active: bool,
    pub timeout_us: u32,
}

/// `read`/`write` status byte: 0 on success, 0xFF on failure (cpp:262/276).
pub const BOARD_STATUS_OK: u8 = 0x00;
pub const BOARD_STATUS_ERR: u8 = 0xFF;

#[derive(Debug)]
pub struct SysGame {
    pub watchdog: WatchdogControl,
    /// 16-byte board storage initialized to all-0xFF (cpp:64 `memset(..., -1)`).
    pub board_storage: [u8; BOARD_STORAGE_SIZE],
    pub board_storage_written: bool,
    pub system_sw_version: u64,
    pub root_perm: bool,

    pub watchdog_start_calls: u64,
    pub watchdog_stop_calls: u64,
    pub watchdog_clear_calls: u64,
    pub set_sw_version_calls: u64,
    pub get_sw_version_calls: u64,
    pub board_read_calls: u64,
    pub board_write_calls: u64,
    pub get_rtc_status_calls: u64,
}

impl Default for SysGame {
    fn default() -> Self {
        Self {
            watchdog: WatchdogControl::default(),
            board_storage: [0xFF; BOARD_STORAGE_SIZE],
            board_storage_written: false,
            system_sw_version: 0,
            root_perm: false,
            watchdog_start_calls: 0,
            watchdog_stop_calls: 0,
            watchdog_clear_calls: 0,
            set_sw_version_calls: 0,
            get_sw_version_calls: 0,
            board_read_calls: 0,
            board_write_calls: 0,
            get_rtc_status_calls: 0,
        }
    }
}

impl SysGame {
    pub fn new() -> Self {
        Self::default()
    }

    /// `_sys_game_watchdog_start(timeout)` — cpp:171-196.
    /// Bit math byte-exato: timeout *= 1_000_000 then & -64 (= !63).
    /// EABORT if already active. Sets needs_restart=true on first start.
    pub fn watchdog_start(&mut self, timeout: u32) -> Result<(), CellError> {
        self.watchdog_start_calls = self.watchdog_start_calls.saturating_add(1);
        if self.watchdog.active {
            return Err(CELL_EABORT);
        }
        let timeout_us = timeout.wrapping_mul(1_000_000) & !63u32;
        self.watchdog.needs_restart = true;
        self.watchdog.active = true;
        self.watchdog.timeout_us = timeout_us;
        Ok(())
    }

    /// `_sys_game_watchdog_stop()` — cpp:198-214. No-op if not active.
    pub fn watchdog_stop(&mut self) -> Result<(), CellError> {
        self.watchdog_stop_calls = self.watchdog_stop_calls.saturating_add(1);
        if self.watchdog.active {
            self.watchdog.active = false;
        }
        Ok(())
    }

    /// `_sys_game_watchdog_clear()` — cpp:216-232. Sets needs_restart=true
    /// only if active AND not already needing restart (idempotent).
    pub fn watchdog_clear(&mut self) -> Result<(), CellError> {
        self.watchdog_clear_calls = self.watchdog_clear_calls.saturating_add(1);
        if self.watchdog.active && !self.watchdog.needs_restart {
            self.watchdog.needs_restart = true;
        }
        Ok(())
    }

    /// `_sys_game_set_system_sw_version(version)` — cpp:234-244.
    /// Requires root permission else CELL_ENOSYS.
    pub fn set_system_sw_version(&mut self, version: u64) -> Result<(), CellError> {
        self.set_sw_version_calls = self.set_sw_version_calls.saturating_add(1);
        if !self.root_perm {
            return Err(CELL_ENOSYS);
        }
        self.system_sw_version = version;
        Ok(())
    }

    /// `_sys_game_get_system_sw_version()` — cpp:246-251.
    pub fn get_system_sw_version(&mut self) -> u64 {
        self.get_sw_version_calls = self.get_sw_version_calls.saturating_add(1);
        self.system_sw_version
    }

    /// `_sys_game_board_storage_read(buffer, status)` — cpp:253-265.
    /// Null buffer or status = EFAULT. Otherwise copies 16-byte storage and
    /// writes 0x00 to status.
    pub fn board_storage_read(
        &mut self,
        buffer_out: Option<&mut [u8; BOARD_STORAGE_SIZE]>,
        status_out: Option<&mut u8>,
    ) -> Result<(), CellError> {
        self.board_read_calls = self.board_read_calls.saturating_add(1);
        let buf = buffer_out.ok_or(CELL_EFAULT)?;
        let stat = status_out.ok_or(CELL_EFAULT)?;
        *buf = self.board_storage;
        *stat = BOARD_STATUS_OK;
        Ok(())
    }

    /// `_sys_game_board_storage_write(buffer, status)` — cpp:267-279.
    pub fn board_storage_write(
        &mut self,
        buffer_in: Option<&[u8; BOARD_STORAGE_SIZE]>,
        status_out: Option<&mut u8>,
    ) -> Result<(), CellError> {
        self.board_write_calls = self.board_write_calls.saturating_add(1);
        let buf = buffer_in.ok_or(CELL_EFAULT)?;
        let stat = status_out.ok_or(CELL_EFAULT)?;
        self.board_storage = *buf;
        self.board_storage_written = true;
        *stat = BOARD_STATUS_OK;
        Ok(())
    }

    /// `_sys_game_get_rtc_status(status)` — cpp:281-293. Always writes 0.
    pub fn get_rtc_status(
        &mut self,
        status_out: Option<&mut i32>,
    ) -> Result<(), CellError> {
        self.get_rtc_status_calls = self.get_rtc_status_calls.saturating_add(1);
        let s = status_out.ok_or(CELL_EFAULT)?;
        *s = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entries() {
        assert_eq!(MODULE_NAME, "sys_game");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 8);
    }

    #[test]
    fn errors_byte_exact() {
        assert_eq!(CELL_EFAULT.0, 0x8001_000D);
        assert_eq!(CELL_ENOSYS.0, 0x8001_0001);
        assert_eq!(CELL_EABORT.0, 0x8001_0010);
    }

    #[test]
    fn board_storage_initial_is_all_0xff() {
        let m = SysGame::new();
        assert_eq!(m.board_storage, [0xFF; 16]);
    }

    #[test]
    fn board_storage_size_is_16() {
        assert_eq!(BOARD_STORAGE_SIZE, 16);
    }

    #[test]
    fn watchdog_start_sets_active_with_bit_math() {
        let mut m = SysGame::new();
        m.watchdog_start(2).unwrap();
        assert!(m.watchdog.active);
        assert!(m.watchdog.needs_restart);
        // 2 * 1_000_000 = 2_000_000 (0x1E_8480), &!63 keeps it (2_000_000 & -64).
        // 2_000_000 % 64 = 2_000_000 - 31_250*64 = 2_000_000 - 2_000_000 = 0. Already aligned.
        assert_eq!(m.watchdog.timeout_us, 2_000_000);
    }

    #[test]
    fn watchdog_start_bit_clears_low_6_bits() {
        let mut m = SysGame::new();
        // timeout=63 → 63_000_000 → &!63 must clear low 6 bits.
        // 63_000_000 binary low: ... mod 64 = 63_000_000 - 984_375*64 = 0. Hmm always 0?
        // Let me try a value where low bits aren't zero: timeout=1 → 1_000_000.
        // 1_000_000 mod 64 = 1_000_000 - 15625*64 = 0. Also 0.
        // Actually 1_000_000 = 15625 × 64, so * 1M aligns naturally to 64.
        // The mask is paranoid — preserves cpp:177 byte-exato regardless.
        m.watchdog_start(1).unwrap();
        assert_eq!(m.watchdog.timeout_us, 1_000_000);
    }

    #[test]
    fn watchdog_double_start_eabort() {
        let mut m = SysGame::new();
        m.watchdog_start(1).unwrap();
        assert_eq!(m.watchdog_start(2), Err(CELL_EABORT));
    }

    #[test]
    fn watchdog_stop_clears_active() {
        let mut m = SysGame::new();
        m.watchdog_start(1).unwrap();
        m.watchdog_stop().unwrap();
        assert!(!m.watchdog.active);
    }

    #[test]
    fn watchdog_stop_when_inactive_is_noop_ok() {
        let mut m = SysGame::new();
        m.watchdog_stop().unwrap();
        assert!(!m.watchdog.active);
    }

    #[test]
    fn watchdog_clear_only_active_and_not_already_needing_restart() {
        let mut m = SysGame::new();
        // Inactive — no-op.
        m.watchdog_clear().unwrap();
        assert!(!m.watchdog.needs_restart);
        // Start (sets needs_restart=true).
        m.watchdog_start(1).unwrap();
        assert!(m.watchdog.needs_restart);
        // Manually flip needs_restart=false (simulates watchdog tick consumed it).
        m.watchdog.needs_restart = false;
        // Clear sets needs_restart=true.
        m.watchdog_clear().unwrap();
        assert!(m.watchdog.needs_restart);
        // Clear again when already true is no-op (idempotent).
        m.watchdog_clear().unwrap();
        assert!(m.watchdog.needs_restart);
    }

    #[test]
    fn set_sw_version_requires_root() {
        let mut m = SysGame::new();
        assert_eq!(m.set_system_sw_version(123), Err(CELL_ENOSYS));
        m.root_perm = true;
        m.set_system_sw_version(456).unwrap();
        assert_eq!(m.system_sw_version, 456);
    }

    #[test]
    fn get_sw_version_returns_stored() {
        let mut m = SysGame::new();
        m.system_sw_version = 0x12345678_9ABCDEF0;
        assert_eq!(m.get_system_sw_version(), 0x12345678_9ABCDEF0);
    }

    #[test]
    fn board_read_null_efault() {
        let mut m = SysGame::new();
        let mut s = 0u8;
        assert_eq!(m.board_storage_read(None, Some(&mut s)), Err(CELL_EFAULT));
        let mut buf = [0u8; 16];
        assert_eq!(m.board_storage_read(Some(&mut buf), None), Err(CELL_EFAULT));
    }

    #[test]
    fn board_read_writes_initial_0xff() {
        let mut m = SysGame::new();
        let mut buf = [0u8; 16];
        let mut stat = 0xAAu8;
        m.board_storage_read(Some(&mut buf), Some(&mut stat)).unwrap();
        assert_eq!(buf, [0xFF; 16]);
        assert_eq!(stat, BOARD_STATUS_OK);
    }

    #[test]
    fn board_write_persists() {
        let mut m = SysGame::new();
        let payload = [0xAB; 16];
        let mut stat = 0xFFu8;
        m.board_storage_write(Some(&payload), Some(&mut stat)).unwrap();
        assert_eq!(m.board_storage, payload);
        assert!(m.board_storage_written);
        assert_eq!(stat, BOARD_STATUS_OK);
        // Read back.
        let mut buf = [0u8; 16];
        m.board_storage_read(Some(&mut buf), Some(&mut stat)).unwrap();
        assert_eq!(buf, payload);
    }

    #[test]
    fn board_write_null_efault() {
        let mut m = SysGame::new();
        let mut stat = 0u8;
        assert_eq!(m.board_storage_write(None, Some(&mut stat)), Err(CELL_EFAULT));
    }

    #[test]
    fn get_rtc_status_writes_zero() {
        let mut m = SysGame::new();
        let mut s = 0xAA;
        m.get_rtc_status(Some(&mut s)).unwrap();
        assert_eq!(s, 0);
    }

    #[test]
    fn get_rtc_status_null_efault() {
        let mut m = SysGame::new();
        assert_eq!(m.get_rtc_status(None), Err(CELL_EFAULT));
    }

    #[test]
    fn full_sys_game_lifecycle_smoke() {
        let mut m = SysGame::new();
        // Boot: query sw version (initially 0).
        assert_eq!(m.get_system_sw_version(), 0);
        // Without root, set fails.
        assert_eq!(m.set_system_sw_version(0x484000), Err(CELL_ENOSYS));
        m.root_perm = true;
        m.set_system_sw_version(0x484000).unwrap();

        // Read board storage (initial 0xFF).
        let mut buf = [0u8; 16];
        let mut stat = 0xAAu8;
        m.board_storage_read(Some(&mut buf), Some(&mut stat)).unwrap();
        assert_eq!(buf, [0xFF; 16]);

        // Write some custom config (16 bytes).
        let mut cfg = [0u8; 16];
        cfg[0] = 0x55;
        cfg[1] = 0x66;
        cfg[2] = 0x77;
        cfg[3] = 0x88;
        m.board_storage_write(Some(&cfg), Some(&mut stat)).unwrap();

        // Start watchdog with 5s timeout.
        m.watchdog_start(5).unwrap();
        assert!(m.watchdog.active);

        // Game keeps watchdog alive periodically.
        for _ in 0..3 {
            m.watchdog.needs_restart = false;
            m.watchdog_clear().unwrap();
            assert!(m.watchdog.needs_restart);
        }

        // Game shuts down.
        m.watchdog_stop().unwrap();
        assert!(!m.watchdog.active);

        // RTC check.
        let mut rtc = 0i32;
        m.get_rtc_status(Some(&mut rtc)).unwrap();
        assert_eq!(rtc, 0);
    }
}
