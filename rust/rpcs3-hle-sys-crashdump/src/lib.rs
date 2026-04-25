//! `rpcs3-hle-sys-crashdump` — PS3 crash-dump user-log-area HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/sys_crashdump.cpp` + `.h`.  The
//! firmware exposes 128 "log area" slots the game can register so the
//! system bug-reporter can include them in a crash dump.  Only two
//! entry points are registered in RPCS3 (`get_user_log_area` /
//! `set_user_log_area`) and both are stubs after argument validation.
//! The Rust port adds a real 128-slot registry so higher layers can
//! drive the CRASH flow end-to-end.
//!
//! ## Entry points covered
//!
//! | C++ function                        | Rust wrapper                       |
//! |-------------------------------------|------------------------------------|
//! | `sys_crash_dump_get_user_log_area`  | [`CrashDump::get_user_log_area`]   |
//! | `sys_crash_dump_set_user_log_area`  | [`CrashDump::set_user_log_area`]   |

extern crate alloc;

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes
// =====================================================================

pub const CELL_EINVAL: CellError = CellError(0x8001_0002);

// =====================================================================
// Constants — byte-exact with sys_crashdump.h
// =====================================================================

/// `SYS_CRASH_DUMP_MAX_LABEL_SIZE = 16` — "15 + 1 (0 terminated)" per
/// the header comment.
pub const SYS_CRASH_DUMP_MAX_LABEL_SIZE: usize = 16;

/// `SYS_CRASH_DUMP_MAX_LOG_AREA = 127` — index is validated with
/// `index > MAX_LOG_AREA`, so valid indices are `0..=127` (128 slots
/// total).
pub const SYS_CRASH_DUMP_MAX_LOG_AREA: u8 = 127;

/// Number of log-area slots the firmware preallocates.
pub const NUM_LOG_AREAS: usize = SYS_CRASH_DUMP_MAX_LOG_AREA as usize + 1;

// =====================================================================
// Log-area record
// =====================================================================

/// Mirror of `sys_crash_dump_log_area_info_t`.  The label is a 16-byte
/// NUL-terminated buffer; `addr` is a guest pointer, `size` is a plain
/// `u32` — matches the BE struct on-guest but stored host-endian here
/// for tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogAreaInfo {
    pub label: [u8; SYS_CRASH_DUMP_MAX_LABEL_SIZE],
    pub addr: u32,
    pub size: u32,
}

impl Default for LogAreaInfo {
    fn default() -> Self {
        Self { label: [0; SYS_CRASH_DUMP_MAX_LABEL_SIZE], addr: 0, size: 0 }
    }
}

impl LogAreaInfo {
    /// Build a log-area record from a label + addr + size.  Truncates
    /// `label` to `MAX_LABEL_SIZE - 1` bytes and always NUL-terminates.
    #[must_use]
    pub fn new(label: &str, addr: u32, size: u32) -> Self {
        let mut buf = [0u8; SYS_CRASH_DUMP_MAX_LABEL_SIZE];
        let bytes = label.as_bytes();
        let take = bytes.len().min(SYS_CRASH_DUMP_MAX_LABEL_SIZE - 1);
        buf[..take].copy_from_slice(&bytes[..take]);
        // Byte 15 always stays 0 (NUL terminator).
        Self { label: buf, addr, size }
    }

    /// Return the label as a `&str`, stripping the NUL and everything
    /// after.  Returns an empty slice if the buffer is all-zero.
    #[must_use]
    pub fn label_str(&self) -> &str {
        let end = self.label.iter().position(|&b| b == 0).unwrap_or(self.label.len());
        // UTF-8 validity is not enforced by the firmware — label is
        // typically ASCII.  Fall back to empty on invalid UTF-8.
        core::str::from_utf8(&self.label[..end]).unwrap_or("")
    }
}

// =====================================================================
// Crash-dump registry
// =====================================================================

/// Fixed-size array of 128 slots (indices `0..=127`) matching
/// `SYS_CRASH_DUMP_MAX_LOG_AREA`.  Every slot starts empty; callers
/// populate them via [`Self::set_user_log_area`] and read them back
/// with [`Self::get_user_log_area`].
#[derive(Debug, Clone)]
pub struct CrashDump {
    slots: [LogAreaInfo; NUM_LOG_AREAS],
}

impl Default for CrashDump {
    fn default() -> Self {
        Self { slots: [LogAreaInfo::default(); NUM_LOG_AREAS] }
    }
}

impl CrashDump {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Port of `sys_crash_dump_get_user_log_area` (cpp:7-17).
    ///
    /// # Errors
    /// * [`CELL_EINVAL`] if `index > SYS_CRASH_DUMP_MAX_LOG_AREA` or
    ///   `entry_valid` is false.
    pub fn get_user_log_area(
        &self,
        index: u8,
        entry_valid: bool,
    ) -> Result<LogAreaInfo, CellError> {
        if index > SYS_CRASH_DUMP_MAX_LOG_AREA || !entry_valid {
            return Err(CELL_EINVAL);
        }
        Ok(self.slots[index as usize])
    }

    /// Port of `sys_crash_dump_set_user_log_area` (cpp:19-29).
    ///
    /// # Errors
    /// * [`CELL_EINVAL`] if `index > SYS_CRASH_DUMP_MAX_LOG_AREA` or
    ///   `new_entry_valid` is false.
    pub fn set_user_log_area(
        &mut self,
        index: u8,
        new_entry_valid: bool,
        new_entry: LogAreaInfo,
    ) -> Result<(), CellError> {
        if index > SYS_CRASH_DUMP_MAX_LOG_AREA || !new_entry_valid {
            return Err(CELL_EINVAL);
        }
        self.slots[index as usize] = new_entry;
        Ok(())
    }

    /// Returns the number of non-empty slots (any slot whose
    /// `size != 0` counts as populated).
    #[must_use]
    pub fn populated_count(&self) -> usize {
        self.slots.iter().filter(|s| s.size != 0).count()
    }
}

// =====================================================================
// Registry
// =====================================================================

pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sys_crash_dump_get_user_log_area",
    "sys_crash_dump_set_user_log_area",
];

#[must_use]
pub fn is_registered(name: &str) -> bool {
    REGISTERED_ENTRY_POINTS.contains(&name)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- constants ---------------------------------------------------

    #[test]
    fn constants_byte_exact() {
        assert_eq!(CELL_EINVAL.0, 0x8001_0002);
        assert_eq!(SYS_CRASH_DUMP_MAX_LABEL_SIZE, 16);
        assert_eq!(SYS_CRASH_DUMP_MAX_LOG_AREA, 127);
        assert_eq!(NUM_LOG_AREAS, 128);
    }

    // ---- LogAreaInfo ------------------------------------------------

    #[test]
    fn log_area_new_short_label() {
        let info = LogAreaInfo::new("boot", 0x1000, 0x200);
        assert_eq!(&info.label[..4], b"boot");
        assert_eq!(info.label[4], 0);
        assert_eq!(info.addr, 0x1000);
        assert_eq!(info.size, 0x200);
    }

    #[test]
    fn log_area_new_truncates_long_label() {
        // 16-char label — should keep 15 bytes + NUL terminator.
        let info = LogAreaInfo::new("0123456789ABCDEF", 0, 0);
        assert_eq!(&info.label[..15], b"0123456789ABCDE");
        assert_eq!(info.label[15], 0);
    }

    #[test]
    fn log_area_new_18_char_still_nul_terminated() {
        let info = LogAreaInfo::new("123456789012345678", 0, 0);
        assert_eq!(info.label[15], 0);
    }

    #[test]
    fn log_area_new_empty_label() {
        let info = LogAreaInfo::new("", 0, 0);
        assert_eq!(info.label, [0; 16]);
    }

    #[test]
    fn log_area_label_str_strips_nul() {
        let info = LogAreaInfo::new("hello", 0, 0);
        assert_eq!(info.label_str(), "hello");
    }

    #[test]
    fn log_area_label_str_empty_on_zero_buffer() {
        let info = LogAreaInfo::default();
        assert_eq!(info.label_str(), "");
    }

    #[test]
    fn log_area_default_is_zeroed() {
        let info = LogAreaInfo::default();
        assert_eq!(info.addr, 0);
        assert_eq!(info.size, 0);
        assert_eq!(info.label, [0; 16]);
    }

    // ---- get_user_log_area ------------------------------------------

    #[test]
    fn get_index_128_is_einval() {
        let cd = CrashDump::new();
        assert_eq!(cd.get_user_log_area(128, true).unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn get_index_255_is_einval() {
        let cd = CrashDump::new();
        assert_eq!(cd.get_user_log_area(255, true).unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn get_null_entry_is_einval() {
        let cd = CrashDump::new();
        assert_eq!(cd.get_user_log_area(0, false).unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn get_fresh_slot_is_zero() {
        let cd = CrashDump::new();
        let info = cd.get_user_log_area(7, true).unwrap();
        assert_eq!(info, LogAreaInfo::default());
    }

    #[test]
    fn get_index_boundary_127_ok() {
        let cd = CrashDump::new();
        assert!(cd.get_user_log_area(127, true).is_ok());
    }

    #[test]
    fn get_index_zero_ok() {
        let cd = CrashDump::new();
        assert!(cd.get_user_log_area(0, true).is_ok());
    }

    // ---- set_user_log_area ------------------------------------------

    #[test]
    fn set_index_128_is_einval() {
        let mut cd = CrashDump::new();
        assert_eq!(
            cd.set_user_log_area(128, true, LogAreaInfo::default()).unwrap_err(),
            CELL_EINVAL,
        );
    }

    #[test]
    fn set_null_entry_is_einval() {
        let mut cd = CrashDump::new();
        assert_eq!(
            cd.set_user_log_area(0, false, LogAreaInfo::default()).unwrap_err(),
            CELL_EINVAL,
        );
    }

    #[test]
    fn set_then_get_roundtrips() {
        let mut cd = CrashDump::new();
        let entry = LogAreaInfo::new("gamelog", 0x4000_0000, 0x1000);
        cd.set_user_log_area(42, true, entry).unwrap();
        let got = cd.get_user_log_area(42, true).unwrap();
        assert_eq!(got, entry);
    }

    #[test]
    fn set_boundary_index_127_ok() {
        let mut cd = CrashDump::new();
        let entry = LogAreaInfo::new("last", 0x8000, 0x100);
        cd.set_user_log_area(127, true, entry).unwrap();
        assert_eq!(cd.get_user_log_area(127, true).unwrap(), entry);
    }

    #[test]
    fn set_overwrites_existing() {
        let mut cd = CrashDump::new();
        cd.set_user_log_area(0, true, LogAreaInfo::new("old", 1, 1)).unwrap();
        cd.set_user_log_area(0, true, LogAreaInfo::new("new", 2, 2)).unwrap();
        let got = cd.get_user_log_area(0, true).unwrap();
        assert_eq!(got.label_str(), "new");
        assert_eq!(got.addr, 2);
    }

    #[test]
    fn set_does_not_affect_other_slots() {
        let mut cd = CrashDump::new();
        cd.set_user_log_area(5, true, LogAreaInfo::new("five", 0x5000, 0x50)).unwrap();
        let other = cd.get_user_log_area(6, true).unwrap();
        assert_eq!(other, LogAreaInfo::default());
    }

    // ---- populated_count --------------------------------------------

    #[test]
    fn populated_count_empty_is_zero() {
        let cd = CrashDump::new();
        assert_eq!(cd.populated_count(), 0);
    }

    #[test]
    fn populated_count_after_setting_three() {
        let mut cd = CrashDump::new();
        cd.set_user_log_area(0, true, LogAreaInfo::new("a", 0x1000, 0x100)).unwrap();
        cd.set_user_log_area(10, true, LogAreaInfo::new("b", 0x2000, 0x200)).unwrap();
        cd.set_user_log_area(127, true, LogAreaInfo::new("c", 0x3000, 0x300)).unwrap();
        assert_eq!(cd.populated_count(), 3);
    }

    #[test]
    fn populated_count_ignores_zero_size_entries() {
        let mut cd = CrashDump::new();
        cd.set_user_log_area(0, true, LogAreaInfo::new("x", 0x1000, 0)).unwrap();
        assert_eq!(cd.populated_count(), 0);
    }

    // ---- registry ---------------------------------------------------

    #[test]
    fn registry_has_two_entries() {
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 2);
    }

    #[test]
    fn registry_contains_both() {
        assert!(is_registered("sys_crash_dump_get_user_log_area"));
        assert!(is_registered("sys_crash_dump_set_user_log_area"));
    }

    #[test]
    fn registry_rejects_unknown() {
        assert!(!is_registered("sys_crash_dump_clear"));
    }

    // ---- full smoke --------------------------------------------------

    #[test]
    fn full_crashdump_lifecycle_smoke() {
        let mut cd = CrashDump::new();
        // 1. Fresh — no areas registered.
        assert_eq!(cd.populated_count(), 0);

        // 2. Populate 3 areas.
        cd.set_user_log_area(0,  true, LogAreaInfo::new("audio",    0x4000_0000, 0x1000)).unwrap();
        cd.set_user_log_area(1,  true, LogAreaInfo::new("graphics", 0x4001_0000, 0x2000)).unwrap();
        cd.set_user_log_area(64, true, LogAreaInfo::new("netlog",   0x4100_0000, 0x400)).unwrap();

        // 3. Readback.
        let a = cd.get_user_log_area(0,  true).unwrap();
        let g = cd.get_user_log_area(1,  true).unwrap();
        let n = cd.get_user_log_area(64, true).unwrap();
        assert_eq!(a.label_str(), "audio");
        assert_eq!(g.label_str(), "graphics");
        assert_eq!(n.label_str(), "netlog");
        assert_eq!(cd.populated_count(), 3);

        // 4. Out-of-bounds + null checks.
        assert_eq!(
            cd.set_user_log_area(200, true, LogAreaInfo::default()).unwrap_err(),
            CELL_EINVAL,
        );
        assert_eq!(cd.get_user_log_area(128, true).unwrap_err(), CELL_EINVAL);
        assert_eq!(cd.get_user_log_area(0, false).unwrap_err(), CELL_EINVAL);

        // 5. Overwrite clears the old label.
        cd.set_user_log_area(0, true, LogAreaInfo::new("AUDIO2", 0x4000_1000, 0x2000)).unwrap();
        assert_eq!(cd.get_user_log_area(0, true).unwrap().label_str(), "AUDIO2");
    }
}
